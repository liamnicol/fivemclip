//! Making sure ffmpeg dies with us.
//!
//! Windows does not kill child processes when their parent exits. If FiveMClip
//! crashes, is force-quit, or is ended from Task Manager, the ffmpeg it spawned
//! carries on recording to a folder nobody is watching - burning GPU and disk
//! until someone who knows what Task Manager is goes looking for it.
//!
//! Two defences. A Job Object makes the kernel enforce the guarantee, covering
//! even a hard kill of our process. And a sweep at startup clears anything a
//! previous run leaked, including runs of versions that predate the job object.

use std::path::Path;

#[cfg(windows)]
use std::process::Child;

/// Put a child process under the job that dies with this process.
///
/// Best effort: failing to adopt is not a reason to refuse to record, it just
/// means falling back to explicit cleanup on the way out.
#[cfg(windows)]
pub fn adopt(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::AssignProcessToJobObject;

    let Some(job) = job_handle() else { return };
    let process = HANDLE(child.as_raw_handle() as _);
    unsafe {
        let _ = AssignProcessToJobObject(job, process);
    }
}

#[cfg(not(windows))]
pub fn adopt(_child: &std::process::Child) {}

/// The process-wide job, created once.
///
/// The handle is deliberately never closed: the kernel kills everything in the
/// job when the last handle to it goes, which is exactly what should happen
/// when this process ends, however it ends.
#[cfg(windows)]
fn job_handle() -> Option<windows::Win32::Foundation::HANDLE> {
    use std::sync::OnceLock;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    // HANDLE is a raw pointer and so not Send; the numeric value is, and it is
    // valid process-wide for the life of the process.
    static JOB: OnceLock<Option<isize>> = OnceLock::new();

    let created = JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(None, windows::core::PCWSTR::null()).ok()?;

        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
        .ok()?;

        Some(job.0 as isize)
    });

    created.map(|raw| HANDLE(raw as _))
}

/// Kill any ffmpeg left over from a previous run of this app.
///
/// Matched on the executable's full path, not its name: killing every
/// `ffmpeg.exe` on the machine would take out the user's own encodes, OBS, or
/// whatever else happens to be running.
pub fn kill_orphans(ffmpeg: &Path) -> usize {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System};

    let Ok(target) = ffmpeg.canonicalize() else {
        return 0;
    };

    let sys = System::new_with_specifics(
        RefreshKind::nothing().with_processes(ProcessRefreshKind::everything()),
    );

    let mut killed = 0;
    for process in sys.processes().values() {
        let Some(exe) = process.exe() else { continue };
        if exe.canonicalize().map(|p| p == target).unwrap_or(false) && process.kill() {
            killed += 1;
        }
    }
    killed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_that_does_not_exist_kills_nothing() {
        // canonicalize fails, and the sweep must not fall back to matching on
        // name - that would kill unrelated ffmpeg processes.
        assert_eq!(kill_orphans(Path::new("/nonexistent/ffmpeg.exe")), 0);
    }
}
