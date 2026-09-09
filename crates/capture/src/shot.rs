//! Single-frame screen grabs.
//!
//! These run as their own short-lived ffmpeg process rather than pulling a
//! frame out of the replay buffer. Desktop Duplication is per-process, so this
//! happily coexists with the recorder, and it means screenshots work even when
//! the buffer is switched off.

use std::fs;
use std::path::PathBuf;
use std::process::Stdio;

use crate::config::Settings;
use crate::ffmpeg::{self, Pipeline, PIPELINES};

pub fn capture(
    ffmpeg_path: &std::path::Path,
    s: &Settings,
    preferred: Option<&Pipeline>,
) -> Result<PathBuf, String> {
    let dir = s.screenshots_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("could not create screenshots folder: {e}"))?;

    let ext = if s.screenshot_jpeg { "jpg" } else { "png" };
    let out = dir.join(format!(
        "Shot_{}.{ext}",
        chrono::Local::now().format("%Y-%m-%d_%H-%M-%S")
    ));

    // Try Desktop Duplication first, then GDI. Whichever capture method the
    // recorder settled on is a good hint about what works on this machine.
    let dda_first = preferred.map(|p| p.uses_dda).unwrap_or(true);
    let mut order: Vec<bool> = if dda_first {
        vec![true, false]
    } else {
        vec![false, true]
    };
    order.dedup();

    let mut last_error = String::from("no capture method available");
    for use_dda in order {
        let pipeline = PIPELINES
            .iter()
            .find(|p| p.uses_dda == use_dda)
            .expect("a pipeline exists for each capture method");

        match try_capture(ffmpeg_path, s, pipeline, &out) {
            Ok(()) => return Ok(out),
            Err(e) => last_error = e,
        }
    }
    Err(last_error)
}

fn try_capture(
    ffmpeg_path: &std::path::Path,
    s: &Settings,
    pipeline: &Pipeline,
    out: &std::path::Path,
) -> Result<(), String> {
    // Grab at 1 fps: we only want one frame and there is no reason to make the
    // duplication API produce sixty of them first.
    let input_settings = Settings {
        fps: 1,
        ..s.clone()
    };

    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostdin".into(),
        "-y".into(),
    ];
    args.extend(pipeline.video_input_args(&input_settings));

    // Image encoders need frames in system memory regardless of how they were
    // captured, so this chain is not the recorder's.
    let filter = if pipeline.uses_dda {
        "hwdownload,format=bgra"
    } else {
        "null"
    };
    args.extend([
        "-filter_complex".into(),
        format!("[0:v]{filter}[v]"),
        "-map".into(),
        "[v]".into(),
        "-frames:v".into(),
        "1".into(),
        "-update".into(),
        "1".into(),
    ]);
    if s.screenshot_jpeg {
        args.extend([
            "-q:v".into(),
            s.screenshot_quality.to_string(),
            "-pix_fmt".into(),
            "yuvj420p".into(),
        ]);
    }
    args.push(out.to_string_lossy().into_owned());

    let result = ffmpeg::command(ffmpeg_path)
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("could not run ffmpeg: {e}"))?;

    if result.status.success() && out.exists() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&result.stderr);
        Err(err
            .lines()
            .last()
            .unwrap_or("screen capture failed")
            .trim()
            .to_string())
    }
}
