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

/// Frame rate asked of the capture source for a single-frame grab.
///
/// It used to be 1, on the reasoning that producing sixty frames to keep one is
/// waste. That had it exactly backwards: ddagrab *paces* to the rate it is
/// given, so at 1 fps the first frame is up to a second away and ffmpeg holds a
/// Desktop Duplication open across the whole wait - which the game in front of
/// it feels as a freeze. At 60 the first frame arrives in about sixteen
/// milliseconds.
pub const GRAB_FPS: u32 = 60;

/// The capture settings a one-frame grab runs with: the user's, but at the grab
/// rate rather than the recording rate.
fn grab_settings(s: &Settings) -> Settings {
    Settings {
        fps: GRAB_FPS,
        ..s.clone()
    }
}

/// Where the next screenshot should be written, named by the clock.
pub fn next_screenshot_path(s: &Settings) -> Result<PathBuf, String> {
    let dir = s.screenshots_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("could not create screenshots folder: {e}"))?;
    let ext = if s.screenshot_jpeg { "jpg" } else { "png" };
    Ok(dir.join(format!(
        "Shot_{}.{ext}",
        chrono::Local::now().format("%Y-%m-%d_%H-%M-%S")
    )))
}

pub fn capture(
    ffmpeg_path: &std::path::Path,
    s: &Settings,
    preferred: Option<&Pipeline>,
) -> Result<PathBuf, String> {
    let out = next_screenshot_path(s)?;
    capture_to(ffmpeg_path, s, preferred, &out)?;
    Ok(out)
}

/// Grab one full frame to an explicit path. Used both for ordinary screenshots
/// and to freeze the screen behind the region-select overlay.
pub fn capture_to(
    ffmpeg_path: &std::path::Path,
    s: &Settings,
    preferred: Option<&Pipeline>,
    out: &std::path::Path,
) -> Result<(), String> {
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("could not create folder: {e}"))?;
    }

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

        match try_capture(ffmpeg_path, s, pipeline, out) {
            Ok(()) => return Ok(()),
            Err(e) => last_error = e,
        }
    }
    Err(last_error)
}

/// Cut a rectangle out of an already-captured frame.
///
/// Cropping the saved frame rather than re-capturing means what the user
/// selected is exactly what they get - the screen cannot change underneath the
/// selection between the drag and the save.
pub fn crop(
    ffmpeg_path: &std::path::Path,
    s: &Settings,
    source: &std::path::Path,
    out: &std::path::Path,
    (x, y, w, h): (u32, u32, u32, u32),
) -> Result<(), String> {
    if w == 0 || h == 0 {
        return Err("That selection was empty.".into());
    }
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("could not create folder: {e}"))?;
    }

    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostdin".into(),
        "-y".into(),
        "-i".into(),
        source.to_string_lossy().into_owned(),
        "-vf".into(),
        format!("crop={w}:{h}:{x}:{y}"),
        "-frames:v".into(),
        "1".into(),
        "-update".into(),
        "1".into(),
    ];
    let wants_jpeg = out
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"))
        .unwrap_or(false);
    if wants_jpeg {
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
        Err(ffmpeg::explain(&String::from_utf8_lossy(&result.stderr)))
    }
}

/// Scratch file holding the frozen screen while the overlay is open.
pub fn region_frame_path() -> PathBuf {
    std::env::temp_dir().join("fivemclip-region-frame.png")
}

fn try_capture(
    ffmpeg_path: &std::path::Path,
    s: &Settings,
    pipeline: &Pipeline,
    out: &std::path::Path,
) -> Result<(), String> {
    let input_settings = grab_settings(s);

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

#[cfg(test)]
mod tests {
    use super::*;

    /// How long the capture source makes us wait for its first frame, in
    /// milliseconds. This is the number the game feels.
    fn first_frame_wait_ms(fps: u32) -> u32 {
        1000 / fps.max(1)
    }

    /// Pins the fix for a screenshot freezing the game for about a second.
    ///
    /// A low grab rate makes ddagrab wait out the frame interval with a Desktop
    /// Duplication open, which the game in front of it feels. The old value of
    /// 1 fps is a full second of that.
    #[test]
    fn the_grab_does_not_wait_long_enough_to_stall_the_game() {
        let waited = first_frame_wait_ms(GRAB_FPS);
        assert!(waited <= 34, "a grab waits {waited} ms for its first frame");
        // What it used to do, for contrast.
        assert_eq!(first_frame_wait_ms(1), 1000);
    }

    /// The grab rate is the screenshot's alone. Following the recording rate
    /// would make a screenshot slow for exactly the people who chose a low
    /// frame rate because their machine is already struggling.
    #[test]
    fn the_grab_rate_ignores_the_recording_frame_rate() {
        for recording_at in [15, 30, 60, 120, 240] {
            let asked = grab_settings(&Settings {
                fps: recording_at,
                ..Default::default()
            });
            assert_eq!(asked.fps, GRAB_FPS, "recording at {recording_at}");
        }
    }

    /// Everything except the frame rate has to come through, or a grab would
    /// use the wrong monitor or ignore the cursor setting.
    #[test]
    fn a_grab_keeps_the_rest_of_the_settings() {
        let s = Settings {
            fps: 30,
            monitor_index: 2,
            capture_cursor: true,
            ..Default::default()
        };
        let asked = grab_settings(&s);
        assert_eq!(asked.monitor_index, 2);
        assert!(asked.capture_cursor);
    }
}
