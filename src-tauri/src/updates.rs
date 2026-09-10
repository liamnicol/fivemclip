//! In-app updates.
//!
//! Updates are checked on launch and applied only when the user says so.
//! Downloading silently in the background would be fine; restarting a recorder
//! out from under someone mid-session would not, so nothing happens without a
//! click.

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
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

/// Is there a newer release? `None` means up to date, or that the check could
/// not be made - an offline user should see nothing, not an error.
#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Option<UpdateInfo> {
    if portable() {
        return None;
    }
    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            crate::diagnostics::log(format!("updater unavailable: {e}"));
            return None;
        }
    };

    match updater.check().await {
        Ok(Some(update)) => {
            crate::diagnostics::log(format!("update available: {}", update.version));
            Some(UpdateInfo {
                version: update.version.clone(),
                notes: update.body.clone(),
            })
        }
        Ok(None) => None,
        Err(e) => {
            // Being offline is the common case here, and it is not a problem
            // worth putting in front of someone about to record.
            crate::diagnostics::log(format!("update check failed: {e}"));
            None
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
