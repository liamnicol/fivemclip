//! In-app updates.
//!
//! Updates are checked on launch and applied only when the user says so.
//! Downloading silently in the background would be fine; restarting a recorder
//! out from under someone mid-session would not, so nothing happens without a
//! click.

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

/// What a check actually found.
///
/// Four outcomes used to collapse into one `None`, and the banner showed
/// nothing for all of them: up to date, offline, an updater that could not be
/// built, and a portable copy that can never update at all. Silence in every
/// case is indistinguishable from the check being broken - which is exactly how
/// it was reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// A newer release exists.
    Available,
    /// This is the newest release.
    Current,
    /// The update server could not be reached. Usually just offline.
    Unreachable,
    /// A portable copy. Updating would install a second copy elsewhere.
    Portable,
    /// The updater could not be built - no signing key, or a broken config.
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateCheck {
    pub status: Status,
    /// The version running right now, so a manual check can say so.
    pub current: String,
    /// The newer version, when there is one.
    pub version: Option<String>,
    pub notes: Option<String>,
    /// Why it could not be checked. For the log and for a manual check, never
    /// for the banner.
    pub detail: Option<String>,
}

impl UpdateCheck {
    fn new(app: &AppHandle, status: Status) -> Self {
        Self {
            status,
            current: app.package_info().version.to_string(),
            version: None,
            notes: None,
            detail: None,
        }
    }
}

/// A portable copy must not offer an update.
///
/// The update artefact is the NSIS installer. Running it from a portable copy
/// would install a *second*, separate FiveMClip into the user's profile and
/// leave the folder they are actually running untouched - so the banner would
/// reappear on every launch, for ever, having done nothing they asked for.
fn portable() -> bool {
    fivemclip_capture::config::is_portable()
}

/// Look for a newer release, and say what was found either way.
///
/// The caller decides what to do with each outcome: the banner only ever
/// appears for `Available`, while a check the user asked for reports all of
/// them. Pressing a button and getting nothing back is the same bug twice.
#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> UpdateCheck {
    if portable() {
        return UpdateCheck::new(&app, Status::Portable);
    }
    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            crate::diagnostics::log(format!("updater unavailable: {e}"));
            let mut check = UpdateCheck::new(&app, Status::Unsupported);
            check.detail = Some(e.to_string());
            return check;
        }
    };

    match updater.check().await {
        Ok(Some(update)) => {
            crate::diagnostics::log(format!("update available: {}", update.version));
            let mut check = UpdateCheck::new(&app, Status::Available);
            check.version = Some(update.version.clone());
            check.notes = update.body.clone();
            check
        }
        Ok(None) => {
            crate::diagnostics::log("update check: already current");
            UpdateCheck::new(&app, Status::Current)
        }
        Err(e) => {
            // Being offline is the common case, and it is not worth putting in
            // front of someone about to record - so the banner ignores this.
            // A check they pressed a button for says so.
            crate::diagnostics::log(format!("update check failed: {e}"));
            let mut check = UpdateCheck::new(&app, Status::Unreachable);
            check.detail = Some(e.to_string());
            check
        }
    }
}

/// Download and install, then restart into the new version.
///
/// The buffer is stopped first. Replacing the executable underneath a running
/// ffmpeg would leave the installer fighting a locked file, and a session in
/// progress is saved rather than lost.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    if portable() {
        return Err("A portable copy updates by downloading the new zip.".into());
    }
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| format!("Could not reach the update server: {e}"))?
        .ok_or("Already up to date.")?;

    crate::diagnostics::log(format!("installing update {}", update.version));

    {
        let handle = app.clone();
        let stopped = tauri::async_runtime::spawn_blocking(move || {
            use crate::state::AppState;
            use tauri::Manager;

            let state = handle.state::<AppState>();
            let saving = state
                .recorder
                .lock()
                .as_ref()
                .map(|r| r.session_active())
                .unwrap_or(false);
            if saving {
                if let Some(recorder) = state.recorder.lock().as_mut() {
                    let _ = recorder.stop_session();
                }
            }
            state.stop_buffer(true);
        })
        .await;
        if let Err(e) = stopped {
            crate::diagnostics::log(format!("could not stop cleanly before updating: {e}"));
        }
    }

    let mut downloaded = 0usize;
    update
        .download_and_install(
            |chunk, total| {
                downloaded += chunk;
                if let Some(total) = total {
                    crate::diagnostics::log(format!("update: {downloaded}/{total} bytes"));
                }
            },
            || crate::diagnostics::log("update downloaded, installing"),
        )
        .await
        .map_err(|e| format!("Update failed: {e}"))?;

    crate::diagnostics::log("update installed, restarting");
    app.restart();
}
