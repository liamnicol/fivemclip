//! A log file that survives whatever the app does next.
//!
//! Written line by line and flushed immediately rather than buffered: the
//! failures worth diagnosing are the ones where the process is frozen or gets
//! killed from Task Manager, and a buffered log loses exactly the last line -
//! the one naming what it was doing when it stopped.

use std::fmt::Display;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static LOG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

/// How many previous runs to keep alongside the current one.
///
/// One was not enough, and the way it failed is worth remembering: the log that
/// matters is from the run that crashed, and installing a new version restarts
/// the app twice, which pushed that log straight off the end. By the time
/// anyone went looking, the crash had been overwritten by two clean starts.
const KEEP_PREVIOUS: usize = 5;

/// `fivemclip.log.2`, and so on.
fn generation(path: &Path, n: usize) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "fivemclip.log".into());
    path.with_file_name(format!("{name}.{n}"))
}

/// Shuffle the old logs along, oldest dropped.
fn rotate(path: &Path) {
    let _ = std::fs::remove_file(generation(path, KEEP_PREVIOUS));
    for n in (1..KEEP_PREVIOUS).rev() {
        let _ = std::fs::rename(generation(path, n), generation(path, n + 1));
    }
    if path.exists() {
        let _ = std::fs::rename(path, generation(path, 1));
    }
}

/// Did the run that wrote this log stop on purpose?
///
/// `None` when there is no previous log to judge - a first run, rather than a
/// silent one. Only the tail is read: the marker is the last thing written, and
/// a long session's log is not worth loading to find out.
fn ended_cleanly(path: &Path) -> Option<bool> {
    let data = std::fs::read(path).ok()?;
    let tail = &data[data.len().saturating_sub(4096)..];
    Some(String::from_utf8_lossy(tail).contains(CLEAN_EXIT))
}

/// Written on the way out, and looked for on the way back in. A log that simply
/// stops is a crash; a log that ends with this is a quit.
pub const CLEAN_EXIT: &str = "shut down cleanly";

/// Point logging at a file beside the settings, keeping the last few runs so a
/// crash is still readable after a restart or three.
pub fn init(settings_path: &Path) {
    let path = settings_path.with_file_name("fivemclip.log");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    rotate(&path);
    // Judged after rotating, on the file the previous run actually wrote.
    let previous_run = ended_cleanly(&generation(&path, 1));
    let _ = LOG_PATH.set(Some(path));

    // A panic on a background thread otherwise vanishes silently, taking the
    // reason for whatever happens next with it.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log(format!("PANIC {info}"));
        previous(info);
    }));

    log(format!(
        "started {} on {}",
        option_env!("FIVEMCLIP_BUILD").unwrap_or(env!("CARGO_PKG_VERSION")),
        std::env::consts::OS
    ));

    // Said out loud rather than left to be worked out. "Did it crash or did I
    // close it?" is the first question about every one of these, and the answer
    // is already on disk.
    match previous_run {
        Some(false) => log(
            "the previous run did not shut down cleanly - it crashed or was killed. \
             Its log is fivemclip.log.1",
        ),
        Some(true) => log("the previous run shut down cleanly"),
        None => {}
    }
}

pub fn log(message: impl Display) {
    let line = format!(
        "{} [{:?}] {message}\n",
        chrono::Local::now().format("%H:%M:%S%.3f"),
        std::thread::current().name().unwrap_or("unnamed")
    );

    // Always to stderr as well, so `cargo tauri dev` shows it without hunting
    // for the file.
    eprint!("{line}");

    if let Some(Some(path)) = LOG_PATH.get() {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
    }
}

pub fn path() -> Option<PathBuf> {
    LOG_PATH.get().cloned().flatten()
}

/// Spawn a named thread.
///
/// Every line in the log carries its thread name, and a log full of "unnamed"
/// tells you nothing about which piece of work stalled. Naming them is the
/// difference between reading a hang and guessing at one.
pub fn thread(name: &str, body: impl FnOnce() + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name(name.to_string())
        .spawn(body);
    if let Err(e) = spawned {
        // Out of threads is not something to paper over silently.
        log(format!("could not start the {name} thread: {e}"));
    }
}

/// Log entry and exit around something that might not come back.
///
/// The value is in the asymmetry: a "begin" with no matching "end" is the
/// signature of a hang, and names exactly where it hung.
pub fn span<T>(what: &str, body: impl FnOnce() -> T) -> T {
    log(format!("begin {what}"));
    let started = std::time::Instant::now();
    let result = body();
    log(format!(
        "end   {what} ({} ms)",
        started.elapsed().as_millis()
    ));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fivemclip-log-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rotating_shuffles_the_old_logs_along() {
        let dir = dir("rotate");
        let path = dir.join("fivemclip.log");

        for run in 1..=3 {
            std::fs::write(&path, format!("run {run}")).unwrap();
            rotate(&path);
        }

        // The newest previous run is .1, the oldest .3.
        assert_eq!(
            std::fs::read_to_string(generation(&path, 1)).unwrap(),
            "run 3"
        );
        assert_eq!(
            std::fs::read_to_string(generation(&path, 2)).unwrap(),
            "run 2"
        );
        assert_eq!(
            std::fs::read_to_string(generation(&path, 3)).unwrap(),
            "run 1"
        );
        assert!(!path.exists(), "the live log is moved aside, not copied");
    }

    /// The whole point. Installing an update restarts the app twice, and with
    /// one generation that was enough to lose the log of the crash before it.
    #[test]
    fn a_crash_survives_several_restarts() {
        let dir = dir("survives");
        let path = dir.join("fivemclip.log");

        std::fs::write(&path, "the run that crashed").unwrap();
        for _ in 0..4 {
            rotate(&path);
            std::fs::write(&path, "an uneventful run").unwrap();
        }

        let kept: Vec<String> = (1..=KEEP_PREVIOUS)
            .filter_map(|n| std::fs::read_to_string(generation(&path, n)).ok())
            .collect();
        assert!(
            kept.iter().any(|c| c == "the run that crashed"),
            "four restarts should not lose it, kept: {kept:?}"
        );
    }

    #[test]
    fn the_oldest_is_dropped_rather_than_kept_for_ever() {
        let dir = dir("bounded");
        let path = dir.join("fivemclip.log");
        for run in 0..KEEP_PREVIOUS + 4 {
            std::fs::write(&path, format!("run {run}")).unwrap();
            rotate(&path);
        }
        assert!(!generation(&path, KEEP_PREVIOUS + 1).exists());
        assert!(generation(&path, KEEP_PREVIOUS).exists());
    }

    #[test]
    fn a_log_ending_in_the_marker_reads_as_a_clean_exit() {
        let dir = dir("clean");
        let path = dir.join("fivemclip.log");
        std::fs::write(&path, format!("12:00:00 [\"main\"] {CLEAN_EXIT}\n")).unwrap();
        assert_eq!(ended_cleanly(&path), Some(true));
    }

    #[test]
    fn a_log_that_simply_stops_reads_as_a_crash() {
        let dir = dir("crash");
        let path = dir.join("fivemclip.log");
        std::fs::write(
            &path,
            "12:00:00 [\"region-capture\"] begin freezing the screen\n",
        )
        .unwrap();
        assert_eq!(ended_cleanly(&path), Some(false));
    }

    /// A first run has nothing to judge, and must not be reported as a crash.
    #[test]
    fn no_previous_log_is_not_a_crash() {
        let dir = dir("first");
        assert_eq!(ended_cleanly(&dir.join("fivemclip.log")), None);
    }

    /// Only the tail is read, so a long session that ended cleanly is still
    /// recognised however much came before it.
    #[test]
    fn a_long_log_is_still_judged_by_its_ending() {
        let dir = dir("long");
        let path = dir.join("fivemclip.log");
        let mut content = "a busy evening\n".repeat(50_000);
        content.push_str(&format!("12:00:00 [\"main\"] {CLEAN_EXIT}\n"));
        std::fs::write(&path, content).unwrap();
        assert_eq!(ended_cleanly(&path), Some(true));
    }
}
