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
/// Bytes free on whichever volume holds `path`.
///
/// Used to sanity-check the replay buffer size against reality: a 30 minute
/// buffer at 60 Mbit is 13 GB, and finding that out by filling someone's drive
/// is the wrong way round.
pub fn free_space_bytes(path: &std::path::Path) -> Option<u64> {
    use sysinfo::Disks;
    let disks = Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        // Longest matching mount point wins, so D:\Games beats D:\ .
        .filter(|d| path.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

/// Does this process name match one of the executables the user is watching?
///
/// Entries are matched on the executable stem, case-insensitively, with or
/// without `.exe`. FiveM and RedM get a little extra help: the launcher is
/// `FiveM.exe` but the game itself carries its build number, as in
/// `FiveM_b2802_GTAProcess.exe`, and nobody should have to know that.
///
/// Split out from the process scan so it can be tested, because the obvious
/// version of this was wrong invisibly: matching a bare "fivem" prefix also
/// matched `FiveMClip.exe`, so the app detected itself, concluded the game was
/// permanently running, and never turned anything off.
pub fn matches_trigger(raw_name: &str, triggers: &[String]) -> bool {
    let name = raw_name.to_ascii_lowercase();
    let stem = name.strip_suffix(".exe").unwrap_or(name.as_str());
    if stem.is_empty() {
        return false;
    }

    triggers.iter().any(|trigger| {
        let trigger = trigger.trim().to_ascii_lowercase();
        let trigger = trigger.strip_suffix(".exe").unwrap_or(trigger.as_str());
        if trigger.is_empty() {
            return false;
        }
        if stem == trigger {
            return true;
        }
        match trigger {
            "fivem" => stem.ends_with("gtaprocess"),
            "redm" => stem.ends_with("rdr3process"),
            _ => false,
        }
    })
}

/// Is anything the user is watching for currently running?
pub fn is_trigger_running(triggers: &[String]) -> bool {
    use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};

    // Belt and braces: whatever this executable ends up being called, it must
    // never count as the thing it is watching for.
    let own_pid = Pid::from_u32(std::process::id());

    let sys = System::new_with_specifics(
        RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing()),
    );
    sys.processes()
        .iter()
        .any(|(pid, p)| *pid != own_pid && matches_trigger(&p.name().to_string_lossy(), triggers))
}

/// Running executables, heaviest first.
///
/// Ordering by memory is a cheap proxy for "things the user would recognise":
/// games and browsers sort above the hundred background services nobody wants
/// to scroll past when picking what to record.
pub fn running_processes(limit: usize) -> Vec<String> {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System};

    let sys = System::new_with_specifics(
        RefreshKind::nothing().with_processes(ProcessRefreshKind::everything()),
    );

    let mut seen: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for process in sys.processes().values() {
        let name = process.name().to_string_lossy().into_owned();
        if name.is_empty() {
            continue;
        }
        // Several processes often share a name; the biggest is the interesting one.
        let entry = seen.entry(name).or_insert(0);
        *entry = (*entry).max(process.memory());
    }

    let mut names: Vec<(String, u64)> = seen.into_iter().collect();
    names.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    names.into_iter().take(limit).map(|(n, _)| n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fivem() -> Vec<String> {
        vec!["FiveM".to_string(), "RedM".to_string()]
    }

    #[test]
    fn recognises_the_launcher_and_the_game() {
        for name in [
            "FiveM.exe",
            "FiveM_b2802_GTAProcess.exe",
            "FiveM_b3095_GTAProcess.exe",
            "RedM.exe",
            "RedM_b1491_RDR3Process.exe",
        ] {
            assert!(matches_trigger(name, &fivem()), "{name} should match");
        }
    }

    #[test]
    fn is_not_fooled_by_our_own_executable() {
        // The original bug: FiveMClip.exe starts with "fivem".
        assert!(!matches_trigger("FiveMClip.exe", &fivem()));
        assert!(!matches_trigger("fivemclip.exe", &fivem()));
        assert!(!matches_trigger("FiveMClip", &fivem()));
    }

    #[test]
    fn ignores_unrelated_processes() {
        for name in ["chrome.exe", "Discord.exe", "fivem-something.exe", ""] {
            assert!(!matches_trigger(name, &fivem()), "{name} should not match");
        }
    }

    #[test]
    fn matching_ignores_case_and_the_extension() {
        assert!(matches_trigger("FIVEM.EXE", &fivem()));
        assert!(matches_trigger("fivem_b2802_gtaprocess.exe", &fivem()));
        assert!(matches_trigger(
            "Cyberpunk2077.exe",
            &["cyberpunk2077".to_string()]
        ));
        assert!(matches_trigger(
            "Cyberpunk2077.exe",
            &["Cyberpunk2077.exe".to_string()]
        ));
    }

    #[test]
    fn any_app_can_be_a_trigger() {
        // The whole point: nothing here is FiveM-specific.
        let triggers = vec!["RocketLeague.exe".to_string(), "obs64".to_string()];
        assert!(matches_trigger("RocketLeague.exe", &triggers));
        assert!(matches_trigger("obs64.exe", &triggers));
        assert!(!matches_trigger("FiveM.exe", &triggers));
    }

    #[test]
    fn an_empty_trigger_list_matches_nothing() {
        assert!(!matches_trigger("FiveM.exe", &[]));
        // A blank entry must not become a wildcard.
        assert!(!matches_trigger("anything.exe", &["  ".to_string()]));
    }
}
