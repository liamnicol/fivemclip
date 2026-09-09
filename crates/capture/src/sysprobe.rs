//! Small Windows queries: what GPUs exist, what monitors exist, is FiveM up.

/// Display adapter names, used to fingerprint the machine so a cached encoder
/// choice is thrown away when someone swaps a GPU.
#[cfg(windows)]
pub fn gpu_names() -> Vec<String> {
    let mut names = Vec::new();
    for d in enum_display_devices(None) {
        if !names.contains(&d.description) {
            names.push(d.description);
        }
    }
    names
}

#[cfg(not(windows))]
pub fn gpu_names() -> Vec<String> {
    Vec::new()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MonitorInfo {
    /// Index as we enumerated it. Desktop Duplication numbers its outputs
    /// separately, so this is a strong hint rather than a guarantee - the UI
    /// gives the user a preview button to confirm.
    pub index: u32,
    pub name: String,
    pub description: String,
    pub primary: bool,
}

#[cfg(windows)]
struct RawDevice {
    name: String,
    description: String,
    primary: bool,
}

#[cfg(windows)]
fn enum_display_devices(target: Option<&str>) -> Vec<RawDevice> {
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayDevicesW, DISPLAY_DEVICEW, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP,
        DISPLAY_DEVICE_PRIMARY_DEVICE,
    };

    let wide: Option<Vec<u16>> = target.map(|t| {
        t.encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    });

    let mut out = Vec::new();
    let mut index = 0u32;
    loop {
        let mut dd = DISPLAY_DEVICEW {
            cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        let ptr = match &wide {
            Some(w) => PCWSTR(w.as_ptr()),
            None => PCWSTR::null(),
        };
        let ok = unsafe { EnumDisplayDevicesW(ptr, index, &mut dd, 0) };
        if !ok.as_bool() {
            break;
        }
        index += 1;

        let attached = dd.StateFlags.0 & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP.0 != 0;
        if target.is_none() && !attached {
            continue;
        }
        out.push(RawDevice {
            name: wide_to_string(&dd.DeviceName),
            description: wide_to_string(&dd.DeviceString),
            primary: dd.StateFlags.0 & DISPLAY_DEVICE_PRIMARY_DEVICE.0 != 0,
        });

        // Guard against a driver that never reports failure.
        if index > 32 {
            break;
        }
    }
    out
}

#[cfg(windows)]
fn wide_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

#[cfg(windows)]
pub fn monitors() -> Vec<MonitorInfo> {
    enum_display_devices(None)
        .into_iter()
        .enumerate()
        .map(|(i, adapter)| {
            // The adapter's DeviceString is the GPU; the attached monitor's is
            // the actual panel name, which is what a user recognises.
            let monitor_name = enum_display_devices(Some(&adapter.name))
                .into_iter()
                .next()
                .map(|m| m.description)
                .unwrap_or_else(|| "Display".to_string());
            MonitorInfo {
                index: i as u32,
                name: monitor_name,
                description: adapter.description,
                primary: adapter.primary,
            }
        })
        .collect()
}

#[cfg(not(windows))]
pub fn monitors() -> Vec<MonitorInfo> {
    Vec::new()
}

/// FiveM spawns several processes; the launcher is `FiveM.exe` and the actual
/// game is `FiveM_b<build>_GTAProcess.exe`. Matching the prefix covers both,
/// plus RedM for anyone running that.
/// Does this process name belong to the game?
///
/// Split out from the process scan so it can be tested, because the obvious
/// version of this was wrong in a way that was invisible from the outside:
/// matching a bare "fivem" prefix also matches `FiveMClip.exe`. The app
/// detected itself, concluded the game was permanently running, and
/// "only record while FiveM is running" silently never turned anything off.
pub fn is_game_process(raw_name: &str) -> bool {
    let name = raw_name.to_ascii_lowercase();
    let stem = name.strip_suffix(".exe").unwrap_or(name.as_str());

    // The launcher is `FiveM.exe`; the game itself carries its build number,
    // as in `FiveM_b2802_GTAProcess.exe` or `RedM_b1491_RDR3Process.exe`.
    matches!(stem, "fivem" | "redm")
        || stem.ends_with("gtaprocess")
        || stem.ends_with("rdr3process")
}

pub fn is_fivem_running() -> bool {
    use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};

    // Belt and braces alongside the name check: whatever this executable ends
    // up being called, it must never count as the game.
    let own_pid = Pid::from_u32(std::process::id());

    let sys = System::new_with_specifics(
        RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing()),
    );
    sys.processes()
        .iter()
        .any(|(pid, p)| *pid != own_pid && is_game_process(&p.name().to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_the_launcher_and_the_game() {
        assert!(is_game_process("FiveM.exe"));
        assert!(is_game_process("FiveM_b2802_GTAProcess.exe"));
        assert!(is_game_process("FiveM_b3095_GTAProcess.exe"));
        assert!(is_game_process("RedM.exe"));
        assert!(is_game_process("RedM_b1491_RDR3Process.exe"));
    }

    #[test]
    fn is_not_fooled_by_our_own_executable() {
        // The whole bug: FiveMClip.exe starts with "fivem".
        assert!(!is_game_process("FiveMClip.exe"));
        assert!(!is_game_process("fivemclip.exe"));
        assert!(!is_game_process("FiveMClip"));
    }

    #[test]
    fn ignores_unrelated_processes() {
        for name in [
            "chrome.exe",
            "Discord.exe",
            "explorer.exe",
            "FiveMClipHelper.exe",
            "fivem-something-else.exe",
            "",
        ] {
            assert!(
                !is_game_process(name),
                "{name} should not count as the game"
            );
        }
    }

    #[test]
    fn matching_ignores_case() {
        assert!(is_game_process("FIVEM.EXE"));
        assert!(is_game_process("fivem_b2802_gtaprocess.exe"));
    }
}
