use fivemclip_capture::config::Settings;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::commands::notify;
use crate::state::AppState;

#[derive(Debug, Clone, Copy)]
enum Action {
    SaveClip,
    Screenshot,
    RegionShot,
    ToggleBuffer,
}

/// Rebind every hotkey from the current settings. Called at startup and after
/// any settings save, so the old bindings always go first.
pub fn register(app: &AppHandle, settings: &Settings) {
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();

    for (combo, action) in [
        (&settings.hotkey_save_clip, Action::SaveClip),
        (&settings.hotkey_screenshot, Action::Screenshot),
        (&settings.hotkey_region, Action::RegionShot),
        (&settings.hotkey_toggle_buffer, Action::ToggleBuffer),
    ] {
        let combo = combo.trim();
        if combo.is_empty() {
            continue;
        }
        let result = shortcuts.on_shortcut(combo, move |app, _shortcut, event| {
            // The handler fires for press and release; acting on both would run
            // everything twice.
            if event.state() != ShortcutState::Pressed {
                return;
            }
            let app = app.clone();
            // Saving a clip shells out to ffmpeg. Doing that on the hotkey
            // thread would wedge every other shortcut until it finished.
            std::thread::spawn(move || run(&app, action));
        });

        if let Err(e) = result {
            notify(
                app,
                "Hotkey unavailable",
                &format!("{combo} could not be registered - another app may have it. ({e})"),
            );
        }
    }
}

fn run(app: &AppHandle, action: Action) {
    let state = app.state::<AppState>();
    match action {
        Action::SaveClip => {
            let seconds = state.settings.lock().buffer_seconds;
            let saved = {
                let mut guard = state.recorder.lock();
                match guard.as_mut() {
                    Some(r) => r.save_clip(seconds),
                    None => Err("The replay buffer is not running.".into()),
                }
            };
            match saved {
                Ok(path) => notify(
                    app,
                    "Clip saved",
                    &path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                ),
                Err(e) => notify(app, "Could not save clip", &e),
            }
        }
        Action::Screenshot => {
            let settings = state.settings.lock().clone();
            let Ok(ffmpeg) = state.ffmpeg().cloned() else {
                notify(app, "Screenshot failed", "ffmpeg is missing");
                return;
            };
            let pipeline = settings
                .cached_pipeline
                .as_deref()
                .and_then(fivemclip_capture::ffmpeg::pipeline_by_id);

            match fivemclip_capture::shot::capture(&ffmpeg, &settings, pipeline) {
                Ok(path) => {
                    if settings.imgbb_auto_upload && !settings.imgbb_api_key.trim().is_empty() {
                        upload_and_notify(app, &settings.imgbb_api_key, path);
                    } else {
                        notify(
                            app,
                            "Screenshot saved",
                            &path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                        );
                    }
                }
                Err(e) => notify(app, "Screenshot failed", &e),
            }
        }
        Action::RegionShot => {
            // Opening a window has to happen on the main thread, and this
            // handler is already on a worker.
            let app = app.clone();
            let _ = app.clone().run_on_main_thread(move || {
                let state = app.state::<AppState>();
                if let Err(e) = crate::commands::start_region_capture(app.clone(), state) {
                    notify(&app, "Region capture failed", &e);
                }
            });
        }
        Action::ToggleBuffer => {
            let running = state
                .recorder
                .lock()
                .as_mut()
                .map(|r| r.is_running())
                .unwrap_or(false);
            if running {
                state.stop_buffer(true);
                notify(app, "Replay buffer off", "Nothing is being recorded.");
            } else {
                match state.start_buffer() {
                    Ok(()) => notify(app, "Replay buffer on", "Recording the last few minutes."),
                    Err(e) => notify(app, "Could not start recording", &e),
                }
            }
        }
    }
}

fn upload_and_notify(app: &AppHandle, key: &str, path: std::path::PathBuf) {
    let app = app.clone();
    let key = key.to_string();
    tauri::async_runtime::spawn(async move {
        use tauri_plugin_clipboard_manager::ClipboardExt;
        match crate::upload::imgbb(&key, &path).await {
            Ok(result) => {
                let _ = app.clipboard().write_text(result.url);
                notify(&app, "Screenshot uploaded", "Link copied to clipboard");
            }
            Err(e) => notify(&app, "Screenshot saved, upload failed", &e),
        }
    });
}
