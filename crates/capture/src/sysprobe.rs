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
pub fn is_fivem_running() -> bool {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System};
    let sys = System::new_with_specifics(
        RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing()),
    );
    sys.processes().values().any(|p| {
        let name = p.name().to_string_lossy().to_ascii_lowercase();
        name.starts_with("fivem") || name.starts_with("redm")
    })
}
