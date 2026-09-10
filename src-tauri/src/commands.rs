use std::path::PathBuf;

use fivemclip_capture::config::Settings;
use fivemclip_capture::ffmpeg::ProbeReport;
use fivemclip_capture::sysprobe::{self, MonitorInfo};
use fivemclip_capture::{shot, RecorderStatus};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

use crate::library::{self, MediaItem};
use crate::state::AppState;
use crate::upload::{self, ImgbbResult};

pub const REGION_WINDOW: &str = "region";

#[derive(Serialize)]
pub struct Status {
    #[serde(flatten)]
    pub recorder: RecorderStatus,
    pub ffmpeg_found: bool,
    pub fivem_running: bool,
    pub estimated_buffer_bytes: u64,
    pub library_bytes: u64,
    pub free_bytes: Option<u64>,
    pub space: fivemclip_capture::disk::SpaceVerdict,
    pub paused_for_disk: bool,
    pub portable: bool,
    /// So a bug report can say which build it came from without the reporter
    /// having to go looking.
    pub version: &'static str,
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

    // A session's footage lives in the ring folder under the output directory.
    // Moving it mid-session points the recorder at an empty ring, and the save
    // then reports the session as too short and loses the lot.
    if previous.output_dir != settings.output_dir {
        let recording = state
            .recorder
            .lock()
            .as_ref()
            .map(|r| r.session_active())
            .unwrap_or(false);
        if recording {
            return Err("Stop the session recording before changing where clips are saved.".into());
        }
    }
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
            session_active: false,
            session_seconds: 0,
            session_bytes: 0,
        },
    };
    let free = fivemclip_capture::disk::free_for(&settings);
    Status {
        recorder,
        ffmpeg_found: state.ffmpeg.is_some(),
        fivem_running: sysprobe::is_trigger_running(&settings.trigger_processes),
        estimated_buffer_bytes: settings.estimated_buffer_bytes(),
        library_bytes: library::total_size(&settings),
        space: free
            .map(|f| fivemclip_capture::disk::verdict(f, &settings))
            .unwrap_or(fivemclip_capture::disk::SpaceVerdict::Fine),
        free_bytes: free,
        paused_for_disk: state
            .paused_for_disk
            .load(std::sync::atomic::Ordering::Relaxed),
        portable: fivemclip_capture::config::is_portable(),
        // Set by CI to the same label the installer is named with. Falls back
        // to the bare version for a local build, which is the only case where
        // "which build is this" has an obvious answer.
        version: option_env!("FIVEMCLIP_BUILD").unwrap_or(env!("CARGO_PKG_VERSION")),
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

/// Running executables, heaviest first, for the trigger picker.
#[tauri::command]
pub fn running_processes() -> Vec<String> {
    sysprobe::running_processes(60)
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

/// Save the last `seconds` of buffer.
///
/// Async so Tauri runs it off the UI thread: stitching segments is disk-bound
/// and can take a moment, and doing it on the thread that draws the window
/// froze the app while it worked.
#[tauri::command]
pub async fn save_clip(app: AppHandle, seconds: Option<u32>) -> Result<String, String> {
    let handle = app.clone();
    let path = tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let seconds = seconds.unwrap_or_else(|| state.settings.lock().buffer_seconds);
        let mut guard = state.recorder.lock();
        let recorder = guard
            .as_mut()
            .ok_or_else(|| "The replay buffer has not been started yet.".to_string())?;
        recorder.save_clip(seconds)
    })
    .await
    .map_err(|e| format!("saving was interrupted: {e}"))??;

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    notify(&app, "Clip saved", &name);

    let settings = app.state::<AppState>().settings.lock().clone();
    let pruned = fivemclip_capture::disk::prune(&settings);
    if pruned.deleted > 0 {
        notify(
            &app,
            "Old clips removed",
            &format!(
                "{} file(s), {:.1} GB, to stay under your library limit.",
                pruned.deleted,
                pruned.freed_bytes as f64 / 1e9
            ),
        );
    }
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

/// Freeze the screen and open the selection overlay.
///
/// The screen is captured *first* and the overlay then shows that still image,
/// so what the user drags a box around is exactly what they get. Selecting
/// against a live screen would let a notification pop up between the drag and
/// the crop.
#[tauri::command]
pub fn start_region_capture(app: AppHandle) {
    begin_region_capture(app);
}

/// Freeze the screen and open the selection overlay.
///
/// The screen is captured *first* and the overlay then shows that still image,
/// so what the user drags a box around is exactly what they get. Selecting
/// against a live screen would let a notification pop up between the drag and
/// the crop.
///
/// All of it runs off the UI thread. Grabbing a frame means waiting on ffmpeg,
/// and doing that on the thread that draws the window froze the whole app.
pub fn begin_region_capture(app: AppHandle) {
    std::thread::spawn(move || {
        if let Some(existing) = app.get_webview_window(REGION_WINDOW) {
            let _ = existing.show();
            let _ = existing.set_focus();
            return;
        }

        let state = app.state::<AppState>();
        let prepared = (|| -> Result<(), String> {
            let ffmpeg = state.ffmpeg()?.clone();
            let settings = state.settings.lock().clone();
            let pipeline = settings
                .cached_pipeline
                .as_deref()
                .and_then(fivemclip_capture::ffmpeg::pipeline_by_id);
            shot::capture_to(&ffmpeg, &settings, pipeline, &shot::region_frame_path())
        })();

        if let Err(e) = prepared {
            notify(&app, "Could not capture the screen", &e);
            return;
        }

        // Window creation belongs to the main thread.
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let built = tauri::WebviewWindowBuilder::new(
                &handle,
                REGION_WINDOW,
                tauri::WebviewUrl::App("region.html".into()),
            )
            .title("Select a region")
            .fullscreen(true)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .build();

            if let Err(e) = built {
                notify(
                    &handle,
                    "Could not open the selection overlay",
                    &e.to_string(),
                );
            }
        });
    });
}

#[derive(Serialize)]
pub struct RegionFrame {
    /// Plain filesystem path. The front end runs it through Tauri's own
    /// convertFileSrc rather than us hand-building an asset URL, which is one
    /// less place to get Windows path encoding wrong.
    pub path: String,
}

#[tauri::command]
pub fn region_frame(app: AppHandle) -> Result<RegionFrame, String> {
    let frame = shot::region_frame_path();
    if !frame.exists() {
        return Err("The captured frame is missing.".into());
    }
    // Scoped here rather than at startup: the scratch frame only needs to be
    // readable while an overlay is actually open.
    app.asset_protocol_scope().allow_file(&frame).ok();
    Ok(RegionFrame {
        path: frame.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub async fn finish_region_capture(
    app: AppHandle,
    state: State<'_, AppState>,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<String, String> {
    close_region_window(&app);

    let (ffmpeg, settings) = {
        let ffmpeg = state.ffmpeg()?.clone();
        (ffmpeg, state.settings.lock().clone())
    };

    let frame = shot::region_frame_path();
    let out = shot::next_screenshot_path(&settings)?;
    shot::crop(&ffmpeg, &settings, &frame, &out, (x, y, width, height))?;

    let mut notes: Vec<&str> = Vec::new();

    if settings.copy_screenshot_to_clipboard {
        match copy_image_to_clipboard(&app, &ffmpeg, &settings, &out) {
            Ok(()) => notes.push("copied"),
            // Not worth failing the capture over: the file is already saved.
            Err(_) => notes.push("saved to disk only"),
        }
    }

    if settings.imgbb_auto_upload && !settings.imgbb_api_key.trim().is_empty() {
        match upload::imgbb(&settings.imgbb_api_key, &out).await {
            Ok(result) => {
                let _ = app.clipboard().write_text(result.url);
                notes.clear();
                notes.push("link copied");
            }
            Err(_) => notes.push("upload failed"),
        }
    }

    let _ = std::fs::remove_file(&frame);

    if settings.edit_after_region {
        // Straight into redaction: for anyone hiding names or plates, the
        // capture is only half the job and the library is a detour.
        let _ = open_editor(
            app.clone(),
            state.clone(),
            out.to_string_lossy().into_owned(),
        );
    }

    notify(
        &app,
        "Region captured",
        &if notes.is_empty() {
            out.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            notes.join(" · ")
        },
    );
    Ok(out.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn cancel_region_capture(app: AppHandle) {
    close_region_window(&app);
    let _ = std::fs::remove_file(shot::region_frame_path());
}

fn close_region_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(REGION_WINDOW) {
        let _ = window.close();
    }
}

/// Put an image on the clipboard.
///
/// The clipboard takes PNG, so a JPEG capture is transcoded to a scratch file
/// first rather than being re-encoded in place - the saved file keeps whatever
/// format the user asked for.
fn copy_image_to_clipboard(
    app: &AppHandle,
    ffmpeg: &std::path::Path,
    settings: &Settings,
    image: &std::path::Path,
) -> Result<(), String> {
    let is_png = image
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("png"))
        .unwrap_or(false);

    let png = if is_png {
        image.to_path_buf()
    } else {
        let scratch = std::env::temp_dir().join("fivemclip-clipboard.png");
        let png_settings = Settings {
            screenshot_jpeg: false,
            ..settings.clone()
        };
        // A crop of the whole thing is just a format conversion.
        let dimensions = image_dimensions(ffmpeg, image)?;
        shot::crop(
            ffmpeg,
            &png_settings,
            image,
            &scratch,
            (0, 0, dimensions.0, dimensions.1),
        )?;
        scratch
    };

    let loaded = tauri::image::Image::from_path(&png).map_err(|e| e.to_string())?;
    app.clipboard()
        .write_image(&loaded)
        .map_err(|e| e.to_string())
}

fn image_dimensions(
    ffmpeg: &std::path::Path,
    image: &std::path::Path,
) -> Result<(u32, u32), String> {
    let out = fivemclip_capture::ffmpeg::command(ffmpeg)
        .args(["-hide_banner", "-i"])
        .arg(image)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;

    // ffmpeg reports the stream as "... 1920x1080 ..." and exits non-zero
    // because no output was requested; the dimensions are what we are after.
    let text = String::from_utf8_lossy(&out.stderr);
    for token in text.split(|c: char| c.is_whitespace() || c == ',') {
        if let Some((w, h)) = token.split_once('x') {
            if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                if w > 1 && h > 1 {
                    return Ok((w, h));
                }
            }
        }
    }
    Err("could not read the image size".into())
}

/// Begin keeping every segment.
///
/// Async for the same reason as saving: starting a session restarts ffmpeg,
/// and waiting on a process is not work for the thread that draws the window.
#[tauri::command]
pub async fn start_session(app: AppHandle) -> Result<(), String> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let mut guard = state.recorder.lock();
        let recorder = guard
            .as_mut()
            .ok_or_else(|| "The replay buffer has not been started yet.".to_string())?;
        recorder.start_session()
    })
    .await
    .map_err(|e| format!("starting the session was interrupted: {e}"))??;

    notify(
        &app,
        "Session recording started",
        "Everything from here is being kept until you stop.",
    );
    Ok(())
}

#[tauri::command]
pub async fn stop_session(app: AppHandle) -> Result<String, String> {
    let handle = app.clone();
    // Stitching hours of segments is the longest thing this app does; blocking
    // the UI for it would look exactly like a hang.
    let saved = tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let mut guard = state.recorder.lock();
        let recorder = guard
            .as_mut()
            .ok_or_else(|| "No session is being recorded.".to_string())?;
        recorder.stop_session()
    })
    .await
    .map_err(|e| format!("saving the session was interrupted: {e}"))?;

    match saved {
        Ok(path) => {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            notify(&app, "Session saved", &name);

            let settings = app.state::<AppState>().settings.lock().clone();
            let pruned = fivemclip_capture::disk::prune(&settings);
            if pruned.deleted > 0 {
                notify(
                    &app,
                    "Old recordings removed",
                    &format!(
                        "{} file(s), {:.1} GB, to stay under your library limit.",
                        pruned.deleted,
                        pruned.freed_bytes as f64 / 1e9
                    ),
                );
            }
            Ok(path.to_string_lossy().into_owned())
        }
        Err(e) => {
            notify(&app, "Could not save the session", &e);
            Err(e)
        }
    }
}

#[tauri::command]
pub fn discard_session(state: State<AppState>) {
    if let Some(recorder) = state.recorder.lock().as_mut() {
        recorder.discard_session();
    }
}

pub const EDITOR_WINDOW: &str = "editor";

/// Open the redaction editor on a screenshot.
#[tauri::command]
pub fn open_editor(app: AppHandle, state: State<AppState>, path: String) -> Result<(), String> {
    crate::diagnostics::log(format!("open_editor: {path}"));
    let path = PathBuf::from(path);
    if !library::is_managed(&state.settings.lock(), &path) {
        return Err("That file is not in the FiveMClip folders.".into());
    }
    app.asset_protocol_scope().allow_file(&path).ok();
    *state.editor_target.lock() = Some(path.clone());

    if let Some(existing) = app.get_webview_window(EDITOR_WINDOW) {
        // Reuse the window rather than stacking them up, and tell it which
        // image it is now looking at. show() as well as focus: a window that
        // ended up hidden rather than closed would otherwise be focused
        // invisibly and the button would look broken.
        let _ = existing.emit("editor:open", path.to_string_lossy().into_owned());
        let _ = existing.show();
        let _ = existing.unminimize();
        let _ = existing.set_focus();
        return Ok(());
    }

    let built = crate::diagnostics::span("building the editor window", || {
        tauri::WebviewWindowBuilder::new(
            &app,
            EDITOR_WINDOW,
            tauri::WebviewUrl::App("editor.html".into()),
        )
        .title("Hide things - FiveMClip")
        .inner_size(1100.0, 780.0)
        .min_inner_size(640.0, 480.0)
        .build()
    });

    match built {
        Ok(_) => {
            crate::diagnostics::log("editor window created");
            Ok(())
        }
        Err(e) => {
            crate::diagnostics::log(format!("editor window failed: {e}"));
            Err(format!("could not open the editor: {e}"))
        }
    }
}

/// Which image the editor should be showing.
///
/// The window asks on load rather than being told through its URL: WebviewUrl
/// takes a path, so a query string ends up part of the filename, the asset
/// never resolves, and the window renders a blank page.
#[tauri::command]
pub fn editor_target(state: State<AppState>) -> Option<String> {
    crate::diagnostics::log("editor_target: the editor page loaded and ran its JS");
    state
        .editor_target
        .lock()
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Write the edited image back.
///
/// `replace` overwrites the original, which is what redaction usually wants:
/// leaving an unredacted copy on disk defeats the point of having hidden
/// anything. Saving a copy is offered for when the original still matters.
#[tauri::command]
pub fn save_edited_image(
    app: AppHandle,
    state: State<AppState>,
    path: String,
    png_base64: String,
    replace: bool,
) -> Result<String, String> {
    use base64::Engine;

    let source = PathBuf::from(&path);
    let settings = state.settings.lock().clone();
    if !library::is_managed(&settings, &source) {
        return Err("That file is not in the FiveMClip folders.".into());
    }

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(png_base64.as_bytes())
        .map_err(|e| format!("the edited image was malformed: {e}"))?;

    // Always PNG on the way out: a redacted image re-encoded as JPEG picks up
    // ringing around the edges of solid blocks, which is ugly and, at the
    // margins, leaks a hint of what was underneath.
    let target = if replace {
        source.with_extension("png")
    } else {
        let stem = source
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Shot".into());
        source.with_file_name(format!("{stem}_edited.png"))
    };

    std::fs::write(&target, &bytes).map_err(|e| format!("could not save: {e}"))?;

    // Replacing a JPEG leaves the original behind under its old extension,
    // which is exactly the unredacted copy we were trying not to keep.
    if replace && source != target {
        let _ = std::fs::remove_file(&source);
    }

    if settings.copy_screenshot_to_clipboard {
        if let Ok(image) = tauri::image::Image::from_path(&target) {
            let _ = app.clipboard().write_image(&image);
        }
    }
    Ok(target.to_string_lossy().into_owned())
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

/// Open the log file's folder. The log is the first thing to ask for when
/// something misbehaves, so it should not require knowing where AppData is.
#[tauri::command]
pub fn open_log_folder(app: AppHandle) -> Result<(), String> {
    let path = crate::diagnostics::path().ok_or("Logging is not running.")?;
    app.opener()
        .reveal_item_in_dir(path)
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
