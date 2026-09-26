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

/// The ffmpeg arguments that lay several monitors onto one canvas.
///
/// `inputs` is one input's arguments per monitor, in the same order as
/// `monitors`, so a test can hand in colour sources where the real thing hands
/// in ddagrab. Everything else about the graph is identical, which is the
/// point: the placement arithmetic is what goes wrong, and it is the part that
/// can be checked without a GPU.
///
/// gdigrab would span the desktop in one input and needs none of this, but it
/// cannot see a fullscreen-exclusive game - which is most of what anyone
/// screenshots here - so each monitor is grabbed with Desktop Duplication and
/// composed instead.
///
/// `hardware` says whether the frames arrive in GPU memory and need fetching
/// out before `overlay`, which is a software filter. It is always true in the
/// app; the tests pass false because `hwupload` needs a device that a machine
/// with no GPU cannot provide. So the placement arithmetic is checked against a
/// real ffmpeg and the download step is not - it is one filter, shared with
/// every software pipeline in `ffmpeg::PIPELINES`, and the offsets are what
/// actually go wrong.
pub fn compose_args(
    monitors: &[crate::sysprobe::MonitorRect],
    inputs: &[Vec<String>],
    hardware: bool,
    out: &std::path::Path,
) -> Result<Vec<String>, String> {
    if monitors.is_empty() || monitors.len() != inputs.len() {
        return Err("Nothing to compose.".into());
    }
    let (left, top, width, height) =
        crate::sysprobe::virtual_bounds(monitors).ok_or("Could not measure the monitors.")?;

    let mut a: Vec<String> = vec!["-hide_banner".into(), "-loglevel".into(), "error".into()];
    a.extend(["-y".into()]);

    // Input 0 is the canvas the monitors are laid onto. Sized to the whole
    // virtual desktop, so a gap between two screens stays a gap rather than
    // sliding the right-hand one leftwards.
    a.extend([
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!("color=c=black:s={width}x{height}:d=1"),
    ]);
    for input in inputs {
        a.extend(input.iter().cloned());
    }

    // Each monitor is overlaid at its own offset from the top-left of the
    // desktop, which is not the origin when a screen sits left of or above the
    // primary - those coordinates are negative.
    let mut graph = String::new();
    let mut last = "0:v".to_string();
    for (i, m) in monitors.iter().enumerate() {
        let input = i + 1;
        let label = format!("m{i}");
        // Out of hardware memory before it can be overlaid: ddagrab hands back
        // D3D11 frames and overlay is a software filter. Skipped when the
        // frames are already in system memory, which is the only way a machine
        // with no GPU can exercise the rest of this.
        let fetch = if hardware {
            "hwdownload,format=bgra"
        } else {
            "format=bgra"
        };
        graph.push_str(&format!("[{input}:v]{fetch}[{label}];"));
        let next = if i + 1 == monitors.len() {
            "out".to_string()
        } else {
            format!("s{i}")
        };
        graph.push_str(&format!(
            "[{last}][{label}]overlay=x={}:y={}[{next}];",
            m.x - left,
            m.y - top,
        ));
        last = next;
    }
    graph.pop();

    a.extend([
        "-filter_complex".into(),
        graph,
        "-map".into(),
        "[out]".into(),
        "-frames:v".into(),
        "1".into(),
        out.to_string_lossy().into_owned(),
    ]);
    Ok(a)
}

/// One monitor's Desktop Duplication input, for `compose_args`.
fn dda_input(m: &crate::sysprobe::MonitorRect, s: &Settings) -> Vec<String> {
    vec![
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!(
            "ddagrab=output_idx={}:framerate={}:draw_mouse={}",
            m.index,
            GRAB_FPS,
            if s.capture_cursor { 1 } else { 0 }
        ),
    ]
}

/// Grab every monitor at once, laid out as they sit on the desktop.
///
/// Same two methods in the same order as `capture_to`, and for the same reason:
/// whichever the recorder settled on is what is known to work here, so a
/// machine whose probe ruled out Desktop Duplication should not pay for a
/// failed ffmpeg spawn on every screenshot. The GDI fallback grabs the whole
/// virtual desktop in one input and needs no arithmetic - but it cannot see a
/// fullscreen-exclusive game, which is most of what anyone screenshots here, so
/// it stays the second choice wherever DDA works.
pub fn capture_all_to(
    ffmpeg_path: &std::path::Path,
    s: &Settings,
    preferred: Option<&Pipeline>,
    monitors: &[crate::sysprobe::MonitorRect],
    out: &std::path::Path,
) -> Result<(), String> {
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("could not create folder: {e}"))?;
    }
    if monitors.is_empty() {
        return Err("No monitors were found to capture.".into());
    }

    let dda_first = preferred.map(|p| p.uses_dda).unwrap_or(true);
    let mut order: Vec<bool> = if dda_first {
        vec![true, false]
    } else {
        vec![false, true]
    };
    order.dedup();

    let mut last_error = String::from("no capture method available");
    for use_dda in order {
        let args = if use_dda {
            let inputs: Vec<Vec<String>> = monitors.iter().map(|m| dda_input(m, s)).collect();
            match compose_args(monitors, &inputs, true, out) {
                Ok(args) => args,
                Err(e) => {
                    last_error = e;
                    continue;
                }
            }
        } else {
            gdi_desktop_args(s, monitors, out)?
        };
        match run_args(ffmpeg_path, &args, out) {
            Ok(()) => return Ok(()),
            Err(e) => last_error = e,
        }
    }
    Err(last_error)
}

/// The whole virtual desktop through GDI, in one input.
fn gdi_desktop_args(
    s: &Settings,
    monitors: &[crate::sysprobe::MonitorRect],
    out: &std::path::Path,
) -> Result<Vec<String>, String> {
    let gdi = PIPELINES
        .iter()
        .find(|p| !p.uses_dda)
        .ok_or("no GDI pipeline available")?;
    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
    ];
    let mut input = gdi.video_input_args(&grab_settings(s));
    // Say where the desktop starts rather than trusting the default. gdigrab
    // measures the virtual screen itself, but its offsets default to 0,0 and
    // are added to that origin - and the origin is negative the moment a
    // monitor sits left of or above the primary.
    if let Some((left, top, width, height)) = crate::sysprobe::virtual_bounds(monitors) {
        let at = input.len().saturating_sub(2);
        input.splice(
            at..at,
            [
                "-offset_x".into(),
                left.to_string(),
                "-offset_y".into(),
                top.to_string(),
                "-video_size".into(),
                format!("{width}x{height}"),
            ],
        );
    }
    args.extend(input);
    args.extend([
        "-frames:v".into(),
        "1".into(),
        out.to_string_lossy().into_owned(),
    ]);
    Ok(args)
}

fn run_args(
    ffmpeg_path: &std::path::Path,
    args: &[String],
    out: &std::path::Path,
) -> Result<(), String> {
    let output = ffmpeg::command(ffmpeg_path)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| ffmpeg::spawn_error(ffmpeg_path, &e))?;
    if output.status.success() && out.is_file() {
        return Ok(());
    }
    Err(ffmpeg::explain(&String::from_utf8_lossy(&output.stderr)))
}

/// Whether this capture should span the monitors.
///
/// One monitor is the same picture either way, so it takes the plain path: the
/// composed one is more machinery for no difference, and it is the path with
/// the hardware download in it.
pub fn wants_all_monitors(s: &Settings, monitors: &[crate::sysprobe::MonitorRect]) -> bool {
    s.screenshot_all_monitors && monitors.len() > 1
}

pub fn capture(
    ffmpeg_path: &std::path::Path,
    s: &Settings,
    preferred: Option<&Pipeline>,
    monitors: &[crate::sysprobe::MonitorRect],
) -> Result<PathBuf, String> {
    let out = next_screenshot_path(s)?;
    // Every monitor only when there is more than one to have; on a single
    // screen the composed path is the same picture through more machinery.
    if wants_all_monitors(s, monitors) {
        capture_all_to(ffmpeg_path, s, preferred, monitors, &out)?;
    } else {
        capture_to(ffmpeg_path, s, preferred, &out)?;
    }
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
        .map_err(|e| ffmpeg::spawn_error(ffmpeg_path, &e))?;

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
        .map_err(|e| ffmpeg::spawn_error(ffmpeg_path, &e))?;

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

#[cfg(test)]
mod compose_tests {
    use super::*;
    use crate::sysprobe::MonitorRect;
    use std::path::Path;

    fn rect(index: u32, x: i32, y: i32, width: u32, height: u32) -> MonitorRect {
        MonitorRect {
            index,
            x,
            y,
            width,
            height,
        }
    }

    /// A solid colour standing in for one monitor's Desktop Duplication output,
    /// in system memory - see `compose_args` on why the hardware download
    /// cannot be exercised here.
    fn colour_input(colour: &str, m: &MonitorRect) -> Vec<String> {
        vec![
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            format!("color=c={colour}:s={}x{}:d=1", m.width, m.height),
        ]
    }

    #[test]
    fn nothing_to_compose_is_refused_rather_than_guessed_at() {
        assert!(compose_args(&[], &[], false, Path::new("out.png")).is_err());
        // One monitor, no input for it.
        assert!(
            compose_args(&[rect(0, 0, 0, 100, 100)], &[], false, Path::new("out.png")).is_err()
        );
    }

    #[test]
    fn the_canvas_is_the_whole_desktop_and_offsets_come_off_its_corner() {
        let monitors = [rect(0, 0, 0, 1920, 1080), rect(1, -2560, -200, 2560, 1440)];
        let inputs: Vec<Vec<String>> = monitors.iter().map(|m| colour_input("red", m)).collect();
        let args = compose_args(&monitors, &inputs, false, Path::new("out.png")).unwrap();
        let joined = args.join(" ");

        assert!(joined.contains("color=c=black:s=4480x1440"), "{joined}");
        // The primary is 2560 right of the desktop's left edge and 200 down.
        assert!(joined.contains("overlay=x=2560:y=200"), "{joined}");
        // The left-hand screen sits at the corner itself.
        assert!(joined.contains("overlay=x=0:y=0"), "{joined}");
    }

    /// The whole thing, through a real ffmpeg: two "monitors" of different
    /// sizes, one of them left of and above the origin, composed onto one
    /// canvas. Then the corners are read back to see that each landed where it
    /// was meant to rather than merely that ffmpeg did not complain.
    #[test]
    fn gdi_fallback_starts_at_the_desktop_origin_not_zero() {
        // A monitor left of and above the primary puts the desktop origin at a
        // negative coordinate. gdigrab adds its offsets to the origin it
        // measures, but they default to 0,0 - so the offsets have to be given
        // explicitly or the fallback silently crops the screens the composed
        // path was careful to include.
        let monitors = [
            crate::sysprobe::MonitorRect {
                index: 0,
                x: 0,
                y: 0,
                width: 400,
                height: 200,
            },
            crate::sysprobe::MonitorRect {
                index: 1,
                x: -300,
                y: -400,
                width: 300,
                height: 600,
            },
        ];
        let out = std::path::Path::new("shot.png");
        let args = gdi_desktop_args(&Settings::default(), &monitors, out).expect("args");

        let at = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .map(|i| args[i + 1].clone())
        };
        assert_eq!(at("-offset_x").as_deref(), Some("-300"));
        assert_eq!(at("-offset_y").as_deref(), Some("-400"));
        assert_eq!(at("-video_size").as_deref(), Some("700x600"));

        // The offsets are input options: they only apply if they come before
        // the -i they belong to.
        let i_at = args.iter().position(|a| a == "-i").expect("an input");
        let off_at = args
            .iter()
            .position(|a| a == "-offset_x")
            .expect("an offset");
        assert!(off_at < i_at, "offsets must precede -i, got {args:?}");
    }

    #[test]
    fn composed_monitors_land_where_they_belong() {
        let Some(ffmpeg) = std::env::var_os("FIVEMCLIP_TEST_FFMPEG").map(std::path::PathBuf::from)
        else {
            eprintln!("skipped: set FIVEMCLIP_TEST_FFMPEG");
            return;
        };
        let dir = std::env::temp_dir().join(format!("fivemclip-compose-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("desktop.png");

        // Left screen is portrait and sits above the primary's top edge.
        let monitors = [rect(0, 0, 200, 400, 200), rect(1, -300, 0, 300, 600)];
        let inputs = [
            colour_input("red", &monitors[0]),
            colour_input("blue", &monitors[1]),
        ];
        let args = compose_args(&monitors, &inputs, false, &out).unwrap();
        let output = ffmpeg::command(&ffmpeg)
            .args(&args)
            .output()
            .expect("ffmpeg runs");
        assert!(
            output.status.success() && out.is_file(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        // Canvas is 700x600: the blue portrait screen on the left, the red one
        // 300 across and 200 down, and black where neither reaches.
        // Near enough, not exact: a colour makes a round trip through the PNG
        // encoder and comes back a point or two off. What is being checked is
        // which screen landed where, not the encoder's arithmetic.
        let is = |got: [u8; 3], want: [u8; 3]| {
            got.iter()
                .zip(&want)
                .all(|(g, w)| (*g as i16 - *w as i16).abs() <= 6)
        };
        let at = |x: u32, y: u32| pixel_at(&ffmpeg, &out, x, y);
        let (left_screen, primary, gap) = (at(150, 300), at(500, 300), at(500, 50));
        assert!(is(left_screen, [0, 0, 255]), "left screen: {left_screen:?}");
        assert!(is(primary, [255, 0, 0]), "primary: {primary:?}");
        assert!(is(gap, [0, 0, 0]), "the gap above the primary: {gap:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn pixel_at(ffmpeg: &Path, file: &Path, x: u32, y: u32) -> [u8; 3] {
        let out = ffmpeg::command(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-i",
                &file.to_string_lossy(),
                "-vf",
                &format!("crop=1:1:{x}:{y}"),
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgb24",
                "-",
            ])
            .output()
            .expect("ffmpeg runs");
        let p = out.stdout;
        assert!(p.len() >= 3, "no pixel came back at {x},{y}");
        [p[0], p[1], p[2]]
    }
}
