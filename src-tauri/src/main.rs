// Hide the console window in release. A clipping tool that flashes a terminal
// over a fullscreen game is a non-starter.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod commands;
mod diagnostics;
mod hotkeys;
mod library;
mod state;
mod updates;
mod upload;

use std::sync::atomic::Ordering;
use std::time::Duration;

use fivemclip_capture::{disk, sysprobe};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

use state::AppState;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--minimised"]),
        ))
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::get_status,
            commands::start_buffer,
            commands::stop_buffer,
            commands::list_monitors,
            commands::running_processes,
            commands::disk_free,
            commands::reprobe,
            commands::save_clip,
            commands::start_session,
            commands::stop_session,
            commands::discard_session,
            commands::take_screenshot,
            commands::start_region_capture,
            commands::region_frame,
            commands::finish_region_capture,
            commands::cancel_region_capture,
            commands::library_items,
            commands::delete_item,
            commands::reveal_item,
            commands::open_item,
            commands::open_editor,
            commands::editor_target,
            commands::save_edited_image,
            commands::open_output_folder,
            commands::open_log_folder,
            commands::upload_imgbb,
            commands::youtube_handoff,
            commands::open_url,
            updates::check_for_update,
            updates::install_update,
        ])
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let settings_path = fivemclip_capture::config::settings_path(&config_dir);
            diagnostics::init(&settings_path);
            let app_state = AppState::load(settings_path);
            let settings = app_state.settings.lock().clone();
            app.manage(app_state);

            let handle = app.handle().clone();
            for dir in library::managed_dirs(&settings) {
                let _ = std::fs::create_dir_all(&dir);
                let _ = handle.asset_protocol_scope().allow_directory(&dir, true);
            }

            // Anything ffmpeg-shaped still running from a previous session is
            // ours and orphaned - a crash, a force-quit, or a build from before
            // the job object existed. Matched on the full executable path, so
            // the user's own ffmpeg and anything like OBS are left alone.
            if let Some(ffmpeg) = app.state::<AppState>().ffmpeg.clone() {
                let orphans = fivemclip_capture::reaper::kill_orphans(&ffmpeg);
                if orphans > 0 {
                    log_orphans(orphans);
                }
            }

            hotkeys::register(&handle, &settings);
            autostart::apply(&handle, settings.autostart);
            build_tray(app)?;

            // Launched by the autostart entry, or configured to stay out of the
            // way: go straight to the tray.
            let launched_minimised = std::env::args().any(|a| a == "--minimised");
            if let Some(window) = app.get_webview_window("main") {
                if launched_minimised || settings.start_minimized {
                    let _ = window.hide();
                } else {
                    let _ = window.show();
                }
            }

            spawn_watchdog(handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Only the main window hides instead of closing: closing it must
                // not stop recording, which is the whole point of a background
                // replay buffer.
                //
                // Scoped by label deliberately. Applied to every window it also
                // trapped the editor and the region overlay, which then could
                // not be closed at all - and since a programmatic close() fires
                // this too, Escape on the overlay only hid it, leaving a stale
                // window that swallowed the next capture.
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to start FiveMClip");
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open FiveMClip", true, None::<&str>)?;
    let clip = MenuItem::with_id(app, "clip", "Save clip now", true, None::<&str>)?;
    let folder = MenuItem::with_id(app, "folder", "Open clips folder", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &clip, &folder, &separator, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("FiveMClip");
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "clip" => {
                let app = app.clone();
                std::thread::spawn(move || {
                    let state = app.state::<AppState>();
                    let seconds = state.settings.lock().buffer_seconds;
                    let result = {
                        let mut guard = state.recorder.lock();
                        match guard.as_mut() {
                            Some(r) => r.save_clip(seconds),
                            None => Err("The replay buffer is not running.".to_string()),
                        }
                    };
                    match result {
                        Ok(path) => commands::notify(
                            &app,
                            "Clip saved",
                            &path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                        ),
                        Err(e) => commands::notify(&app, "Could not save clip", &e),
                    }
                });
            }
            "folder" => {
                use tauri_plugin_opener::OpenerExt;
                let dir = app.state::<AppState>().settings.lock().output_dir.clone();
                let _ = std::fs::create_dir_all(&dir);
                let _ = app
                    .opener()
                    .open_path(dir.to_string_lossy().into_owned(), None::<&str>);
            }
            "quit" => {
                diagnostics::log("tray: quit clicked");
                // Cleanup happens off the UI thread and against a deadline.
                // Doing it inline meant a slow ffmpeg shutdown froze the menu
                // that had just been clicked, leaving the app unkillable
                // except from Task Manager - the exact failure the Quit item
                // exists to avoid.
                //
                // Exiting without a clean stop is safe now: ffmpeg is in a job
                // object that the kernel tears down with this process.
                let app = app.clone();
                std::thread::spawn(move || {
                    let deadline = app.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_secs(20));
                        let _ = deadline;
                        std::process::exit(0);
                    });

                    let state = app.state::<AppState>();
                    let saving = state
                        .recorder
                        .lock()
                        .as_ref()
                        .map(|r| r.session_active())
                        .unwrap_or(false);
                    if saving {
                        // Footage the user asked to keep; worth the wait.
                        if let Some(recorder) = state.recorder.lock().as_mut() {
                            let _ = recorder.stop_session();
                        }
                    }
                    state.stop_buffer(true);
                    app.exit(0);
                });
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// Recorded rather than shown: by the time the window exists the user has
/// already been rescued, and a notification about a process they never knew
/// was running would only worry them.
fn log_orphans(count: usize) {
    eprintln!("cleaned up {count} orphaned ffmpeg process(es) from a previous run");
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Keeps the replay buffer aligned with whether FiveM is actually up, so the
/// app can sit in the tray permanently without burning GPU on the desktop.
fn spawn_watchdog(app: tauri::AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(4));

        let state = app.state::<AppState>();
        let settings = state.settings.lock().clone();

        // Nothing runs until the user has been through first-run setup.
        if !settings.setup_complete {
            continue;
        }

        let manually_stopped = state.manually_stopped.load(Ordering::Relaxed);
        let running = state
            .recorder
            .lock()
            .as_mut()
            .map(|r| r.is_running())
            .unwrap_or(false);

        // Disk comes before everything else: there is no point deciding
        // whether FiveM is up if there is nowhere to write.
        let free = disk::free_for(&settings);
        if let Some(free) = free {
            if disk::verdict(free, &settings) == disk::SpaceVerdict::Critical {
                if !state.paused_for_disk.swap(true, Ordering::Relaxed) {
                    if running {
                        state.stop_buffer(false);
                    }
                    // A session's footage lives in the ring, which nothing
                    // prunes, so the pause cannot lift on its own while one is
                    // running. Saying "resumes when there is room" would be a
                    // lie the user waits on.
                    let session_active = state
                        .recorder
                        .lock()
                        .as_ref()
                        .map(|r| r.session_active())
                        .unwrap_or(false);
                    let advice = if session_active {
                        "Your session so far is safe. Save or discard it, or free up space, to carry on."
                    } else {
                        "Recording resumes on its own once there is room."
                    };
                    commands::notify(
                        &app,
                        "Recording stopped - low disk space",
                        &format!(
                            "{:.1} GB free, below your {} GB limit. {advice}",
                            free as f64 / 1e9,
                            settings.min_free_gb
                        ),
                    );
                }
                continue;
            }

            if state.paused_for_disk.load(Ordering::Relaxed) {
                if !disk::may_resume(free, &settings) {
                    continue;
                }
                state.paused_for_disk.store(false, Ordering::Relaxed);
                commands::notify(&app, "Recording resumed", "There is disk space again.");
            }
        }

        let should_run = if settings.only_while_fivem_running {
            sysprobe::is_trigger_running(&settings.trigger_processes)
        } else {
            true
        };

        if should_run && !running && !manually_stopped {
            if let Err(e) = state.start_buffer() {
                commands::notify(&app, "Could not start recording", &e);
                // Back off rather than retrying every four seconds forever.
                state.manually_stopped.store(true, Ordering::Relaxed);
            }
        } else if !should_run && running {
            // Not a manual stop: the buffer should come back when FiveM does.
            state.stop_buffer(false);
        }
    });
}
