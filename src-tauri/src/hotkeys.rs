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
    ToggleSession,
    ToggleBuffer,
    MarkMoment,
}

/// Rebind every hotkey from the current settings. Called at startup and after
/// any settings save, so the old bindings always go first.
///
/// Always on its own thread. The plugin registers by posting to the main thread
/// and blocking on the reply, so calling it *from* the main thread waits for a
/// task that cannot run until the wait ends. Sync Tauri commands run on that
/// thread - the same trap that made the editor window deadlock.
pub fn register(app: &AppHandle, settings: &Settings) {
    let app = app.clone();
    let settings = settings.clone();
    std::thread::spawn(move || {
        crate::diagnostics::span("registering hotkeys", || register_now(&app, &settings));
    });
}

fn register_now(app: &AppHandle, settings: &Settings) {
    let shortcuts = app.global_shortcut();
    if let Err(e) = shortcuts.unregister_all() {
        crate::diagnostics::log(format!("could not clear the old hotkeys: {e}"));
    }

    for (combo, action) in [
        (&settings.hotkey_save_clip, Action::SaveClip),
        (&settings.hotkey_screenshot, Action::Screenshot),
        (&settings.hotkey_region, Action::RegionShot),
        (&settings.hotkey_session, Action::ToggleSession),
        (&settings.hotkey_toggle_buffer, Action::ToggleBuffer),
        (&settings.hotkey_marker, Action::MarkMoment),
    ] {
        let combo = combo.trim();
        if combo.is_empty() {
            crate::diagnostics::log(format!("{action:?}: no hotkey set"));
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

        match result {
            Ok(()) => crate::diagnostics::log(format!("{action:?}: bound to {combo}")),
            Err(e) => {
                crate::diagnostics::log(format!("{action:?}: {combo} failed - {e}"));
                notify(
                    app,
                    "Hotkey unavailable",
                    &unavailable(combo, &e.to_string()),
                );
            }
        }
    }
}

fn run(app: &AppHandle, action: Action) {
    let state = app.state::<AppState>();
    match action {
        Action::SaveClip => {
            let seconds = state.settings.lock().clip_seconds;
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

            let shot = crate::diagnostics::span("grabbing a screenshot", || {
                fivemclip_capture::shot::capture(&ffmpeg, &settings, pipeline)
            });
            match shot {
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
            // Does its own threading: grabs the frame off the UI thread and
            // hops to main only to create the window.
            crate::commands::begin_region_capture(app.clone());
        }
        Action::ToggleSession => {
            let active = state
                .recorder
                .lock()
                .as_ref()
                .map(|r| r.session_active())
                .unwrap_or(false);

            if active {
                let saved = {
                    let mut guard = state.recorder.lock();
                    match guard.as_mut() {
                        Some(r) => r.stop_session(),
                        None => Err("No session is being recorded.".into()),
                    }
                };
                match saved {
                    Ok(saved) => {
                        let state = app.state::<AppState>();
                        state.markers.set(&saved.path, saved.markers.clone());
                        let name = saved
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let marked = match saved.markers.len() {
                            0 => name,
                            1 => format!("{name} · 1 marker"),
                            n => format!("{name} · {n} markers"),
                        };
                        notify(app, "Session saved", &marked);
                    }
                    Err(e) => notify(app, "Could not save the session", &e),
                }
            } else {
                let started = {
                    let mut guard = state.recorder.lock();
                    match guard.as_mut() {
                        Some(r) => r.start_session(),
                        None => Err("Start the replay buffer first.".into()),
                    }
                };
                match started {
                    Ok(()) => notify(
                        app,
                        "Session recording started",
                        "Everything from here is kept until you stop.",
                    ),
                    Err(e) => notify(app, "Could not start the session", &e),
                }
            }
        }
        Action::MarkMoment => {
            let marked = {
                let mut guard = state.recorder.lock();
                match guard.as_mut() {
                    Some(r) => r.mark_session(),
                    None => Err("The replay buffer is not running.".into()),
                }
            };
            match marked {
                Ok(at) => notify(app, "Moment marked", &format!("at {}", clock(at))),
                Err(e) => notify(app, "Nothing to mark", &e),
            }
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

/// h:mm:ss, because a session marker is as likely to be at 2h14m as at 0m12s.
fn clock(seconds: f64) -> String {
    let whole = seconds.max(0.0) as u64;
    let (h, m, s) = (whole / 3600, (whole % 3600) / 60, whole % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
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

/// Why a binding did not take, in terms the user can act on.
///
/// Print Screen gets its own line because Windows 11 hands it to the Snipping
/// Tool by default, and "another app may have it" sends people hunting through
/// their running programs for something that is a Windows setting.
fn unavailable(combo: &str, error: &str) -> String {
    if combo.eq_ignore_ascii_case("printscreen") {
        return "Windows is holding Print Screen for the Snipping Tool. Turn off \
                Settings > Accessibility > Keyboard > \"Use the Print screen key to open \
                Snipping Tool\", then save again."
            .to_string();
    }
    format!("{combo} could not be registered - another app may have it. ({error})")
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use fivemclip_capture::config::Settings;
    use tauri_plugin_global_shortcut::Shortcut;

    fn parses(combo: &str) -> bool {
        Shortcut::from_str(combo).is_ok()
    }

    /// Print Screen is bindable at all - the front end offers it, so the parser
    /// had better take it.
    #[test]
    fn print_screen_is_a_real_accelerator() {
        assert!(parses("PrintScreen"));
        assert!(parses("Ctrl+PrintScreen"));
    }

    #[test]
    fn print_screen_failure_names_the_windows_setting() {
        let message = super::unavailable("PrintScreen", "already registered");
        assert!(message.contains("Snipping Tool"), "{message}");
        assert!(!message.contains("another app"), "{message}");
    }

    #[test]
    fn any_other_failure_keeps_the_general_advice() {
        let message = super::unavailable("Ctrl+F9", "already registered");
        assert!(message.contains("another app"), "{message}");
    }

    #[test]
    fn every_default_hotkey_parses() {
        let s = Settings::default();
        for combo in [
            &s.hotkey_save_clip,
            &s.hotkey_screenshot,
            &s.hotkey_region,
            &s.hotkey_session,
            &s.hotkey_toggle_buffer,
        ] {
            assert!(parses(combo), "default hotkey {combo:?} does not parse");
        }
    }

    /// The front end builds accelerators out of `event.code`. These are the
    /// shapes it produces; if the parser ever stops taking one of them, a user
    /// gets to save a binding that silently never fires.
    #[test]
    fn the_shapes_the_front_end_produces_parse() {
        for combo in [
            "F7",
            "KeyK",
            "Digit4",
            "Numpad5",
            "Space",
            "BracketLeft",
            "Semicolon",
            "ArrowUp",
            "Insert",
            "Ctrl+F9",
            "Ctrl+Shift+KeyS",
            "Shift+Digit1",
            "Alt+KeyX",
            "Ctrl+Alt+Numpad0",
        ] {
            assert!(parses(combo), "{combo:?} does not parse");
        }
    }

    /// What it used to produce, kept as a record of the bug: `event.key` gives
    /// the character rather than the key, and none of these ever registered.
    #[test]
    fn the_shapes_it_used_to_produce_do_not() {
        for combo in ["Clear", " ", "Shift+!", "Shift+@"] {
            assert!(
                !parses(combo),
                "{combo:?} parses now - the mapping can be simplified"
            );
        }
    }
}
