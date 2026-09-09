use std::path::PathBuf;

use fivemclip_capture::config::Settings;
use fivemclip_capture::ffmpeg::ProbeReport;
use fivemclip_capture::sysprobe::{self, MonitorInfo};
use fivemclip_capture::{shot, RecorderStatus};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

use crate::library::{self, MediaItem};
use crate::state::AppState;
use crate::upload::{self, ImgbbResult};

#[derive(Serialize)]
pub struct Status {
    #[serde(flatten)]
    pub recorder: RecorderStatus,
    pub ffmpeg_found: bool,
    pub fivem_running: bool,
    pub estimated_buffer_bytes: u64,
    pub library_bytes: u64,
}

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().clone()
}

#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    state: State<AppState>,
    mut settings: Settings,
) -> Result<Settings, String> {
    settings.clamp();

    let previous = state.settings.lock().clone();
    // The cached encoder choice is only valid for the monitor it was probed
    // against, since a different output can be on a different GPU entirely.
    if previous.monitor_index != settings.monitor_index {
        settings.cached_pipeline = None;
        settings.cached_pipeline_fingerprint = None;
    }
    let capture_changed = previous.fps != settings.fps
        || previous.bitrate_kbps != settings.bitrate_kbps
        || previous.buffer_seconds != settings.buffer_seconds
        || previous.monitor_index != settings.monitor_index
        || previous.capture_cursor != settings.capture_cursor
        || previous.mic_mode != settings.mic_mode
        || previous.mic_gain_db != settings.mic_gain_db
        || previous.system_gain_db != settings.system_gain_db
        || previous.output_dir != settings.output_dir;

    *state.settings.lock() = settings.clone();
    state.persist()?;

    for dir in library::managed_dirs(&settings) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = app.asset_protocol_scope().allow_directory(&dir, true);
    }

    crate::hotkeys::register(&app, &settings);
    crate::autostart::apply(&app, settings.autostart);

    if capture_changed {
        // Roll the buffer so the new settings take effect immediately rather
        // than at the next launch.
        state.ensure_recorder()?;
    }

    Ok(settings)
}

#[tauri::command]
pub fn get_status(state: State<AppState>) -> Status {
    let settings = state.settings.lock().clone();
    let recorder = match state.recorder.lock().as_mut() {
        Some(r) => r.status(),
        None => RecorderStatus {
            running: false,
            seconds_buffered: 0,
            pipeline: "not started".into(),
            has_audio: false,
            warnings: Vec::new(),
        },
    };
    Status {
        recorder,
        ffmpeg_found: state.ffmpeg.is_some(),
        fivem_running: sysprobe::is_fivem_running(),
        estimated_buffer_bytes: settings.estimated_buffer_bytes(),
        library_bytes: library::total_size(&settings),
    }
}

#[tauri::command]
pub fn start_buffer(state: State<AppState>) -> Result<(), String> {
    state.start_buffer()
}

#[tauri::command]
pub fn stop_buffer(state: State<AppState>) {
    state.stop_buffer(true);
}

/// Free space on the volume a candidate output folder lives on. Takes the path
/// rather than reading settings, so the setup screen can check a folder before
/// it has been saved.
#[tauri::command]
pub fn disk_free(path: String) -> Option<u64> {
    let path = PathBuf::from(path);
    // The folder itself may not exist yet; walk up until something does.
    let mut probe = path.as_path();
    loop {
        if probe.exists() {
            return sysprobe::free_space_bytes(probe);
        }
        probe = probe.parent()?;
    }
}

#[tauri::command]
pub fn list_monitors() -> Vec<MonitorInfo> {
    sysprobe::monitors()
}

/// Force a fresh hardware probe. Exposed because a driver update can change
/// which encoders work without anything else about the machine changing.
#[tauri::command]
pub fn reprobe(state: State<AppState>) -> Result<ProbeReport, String> {
    let ffmpeg = state.ffmpeg()?.clone();
    let settings = state.settings.lock().clone();
    let report = fivemclip_capture::ffmpeg::select_pipeline(&ffmpeg, &settings);

    if let Some(chosen) = &report.chosen {
        let mut s = state.settings.lock();
        s.cached_pipeline = Some(chosen.clone());
        s.cached_pipeline_fingerprint = Some(fivemclip_capture::ffmpeg::fingerprint(
            &ffmpeg,
            s.monitor_index,
        ));
        drop(s);
        let _ = state.persist();
    }
    Ok(report)
}

#[tauri::command]
pub fn save_clip(
    app: AppHandle,
    state: State<AppState>,
    seconds: Option<u32>,
) -> Result<String, String> {
    let seconds = seconds.unwrap_or_else(|| state.settings.lock().buffer_seconds);
    let mut guard = state.recorder.lock();
    let recorder = guard
        .as_mut()
        .ok_or("The replay buffer has not been started yet.")?;
    let path = recorder.save_clip(seconds)?;
    drop(guard);

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    notify(&app, "Clip saved", &name);
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn take_screenshot(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let (ffmpeg, settings, pipeline) = {
        let ffmpeg = state.ffmpeg()?.clone();
        let settings = state.settings.lock().clone();
        let pipeline = settings
            .cached_pipeline
            .as_deref()
            .and_then(fivemclip_capture::ffmpeg::pipeline_by_id);
        (ffmpeg, settings, pipeline)
    };

    let path = shot::capture(&ffmpeg, &settings, pipeline)?;
    let path_string = path.to_string_lossy().into_owned();

    if settings.imgbb_auto_upload && !settings.imgbb_api_key.trim().is_empty() {
        match upload::imgbb(&settings.imgbb_api_key, &path).await {
            Ok(result) => {
                let _ = app.clipboard().write_text(result.url.clone());
                notify(&app, "Screenshot uploaded", "Link copied to clipboard");
                return Ok(path_string);
            }
            Err(e) => {
                notify(&app, "Screenshot saved, upload failed", &e);
                return Ok(path_string);
            }
        }
    }

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    notify(&app, "Screenshot saved", &name);
    Ok(path_string)
}

#[tauri::command]
pub fn library_items(state: State<AppState>) -> Vec<MediaItem> {
    library::list(&state.settings.lock())
}

#[tauri::command]
pub fn delete_item(state: State<AppState>, path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    if !library::is_managed(&state.settings.lock(), &path) {
        return Err("That file is not in the FiveMClip folders.".into());
    }
    std::fs::remove_file(&path).map_err(|e| format!("Could not delete: {e}"))
}

#[tauri::command]
pub fn reveal_item(app: AppHandle, path: String) -> Result<(), String> {
    app.opener()
        .reveal_item_in_dir(PathBuf::from(path))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_item(app: AppHandle, path: String) -> Result<(), String> {
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_output_folder(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let dir = state.settings.lock().output_dir.clone();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    app.opener()
        .open_path(dir.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn upload_imgbb(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<ImgbbResult, String> {
    let key = state.settings.lock().imgbb_api_key.clone();
    let result = upload::imgbb(&key, &PathBuf::from(path)).await?;
    let _ = app.clipboard().write_text(result.url.clone());
    Ok(result)
}

/// Put the clip where YouTube's own upload page can reach it in one paste.
#[tauri::command]
pub fn youtube_handoff(app: AppHandle, path: String) -> Result<(), String> {
    // The path on the clipboard is the point: YouTube's file picker accepts a
    // pasted path directly, so this is one Ctrl+V rather than a folder hunt.
    let _ = app.clipboard().write_text(path.clone());
    let _ = app.opener().reveal_item_in_dir(PathBuf::from(&path));
    app.opener()
        .open_url(upload::YOUTUBE_UPLOAD_URL, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    // Only ever used for the handful of help links in the UI.
    if !url.starts_with("https://") {
        return Err("refused to open a non-https link".into());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

pub fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}
