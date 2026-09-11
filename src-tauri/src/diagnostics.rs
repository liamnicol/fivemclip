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

/// Point logging at a file beside the settings, and keep the previous run's
/// log as `.1` so a crash is still readable after a restart.
pub fn init(settings_path: &Path) {
    let path = settings_path.with_file_name("fivemclip.log");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if path.exists() {
        let _ = std::fs::rename(&path, path.with_file_name("fivemclip.log.1"));
    }
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
