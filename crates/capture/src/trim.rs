//! Cutting a saved clip down to the part worth keeping.
//!
//! Two ways, and the difference is not what it looks like.
//!
//! A stream copy can only *start* at a keyframe, and these clips carry one
//! every two seconds - but the output is MP4, and ffmpeg writes an edit list
//! saying where playback really begins. So a copy trim plays back from exactly
//! the right frame while still physically containing up to two seconds of what
//! was cut. Measured: a copy trim asked to drop everything before 5.0s plays
//! from 5.0s, and the same file decoded with `-ignore_editlist` starts at 4.0s
//! and is a second longer.
//!
//! That hidden second is the whole problem. People here trim to *remove*
//! things - a name, a plate, staff chat - and "it is still in the file, some
//! players will show it" is the same trap as pixelating a name. So the default
//! re-encodes, which is slower and costs a generation of quality but leaves
//! nothing behind. Fast is offered because re-encoding a three hour session is
//! not a thing anybody will wait for.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::config::Settings;
use crate::ffmpeg::{self, Pipeline};

/// Refuse to produce a clip shorter than this. Below about a quarter of a
/// second there is nothing to watch, and a zero-length range is usually a
/// handle dragged past its partner rather than a request.
pub const MIN_SECONDS: f64 = 0.25;

/// What to cut, and how.
#[derive(Debug, Clone, Copy)]
pub struct Request {
    /// Seconds from the start of the file.
    pub start: f64,
    pub end: f64,
    /// Write over the source rather than beside it.
    pub replace: bool,
    /// Stream copy instead of re-encoding. Instant, and leaves up to two
    /// seconds of the cut inside the file - see the module note.
    pub fast: bool,
    /// Squeeze the result under this many bytes by lowering the bitrate rather
    /// than by cutting more. Ignored when `fast` is set, because a stream copy
    /// cannot change the bitrate it is copying.
    pub fit_bytes: Option<u64>,
}

/// Audio is a rounding error next to video, but at the bitrates a size limit
/// forces it stops being one, so it is budgeted for explicitly.
pub const AUDIO_KBPS: u32 = 160;

/// Below this the picture is a smear and the clip is not worth sending.
/// Measured against 1080p gameplay, which is what this records.
pub const MIN_USEFUL_KBPS: u32 = 1_500;

/// The video bitrate that fits `bytes` of output into `seconds` of clip.
///
/// Deliberately shy of the limit. A muxer writes headers and an index, and a
/// rate control aims at the target rather than hitting it exactly - landing at
/// 100.4% of a hard cap means the upload is refused and the whole re-encode was
/// wasted.
pub fn bitrate_to_fit(bytes: u64, seconds: f64) -> u32 {
    if seconds <= 0.0 {
        return MIN_USEFUL_KBPS;
    }
    let budget = (bytes as f64 * 0.95 * 8.0) / seconds / 1000.0;
    (budget - AUDIO_KBPS as f64).max(0.0) as u32
}

/// Cut `source` down to `start`..`end`, in seconds from the start of the file.
///
/// Writes beside the source and only then moves the result into place, so a
/// failure part-way through can never leave the original truncated or missing.
pub fn trim(
    ffmpeg_path: &Path,
    settings: &Settings,
    pipeline: Option<&Pipeline>,
    source: &Path,
    request: &Request,
) -> Result<PathBuf, String> {
    let Request {
        start,
        end,
        replace,
        fast,
        fit_bytes,
    } = *request;
    let duration = end - start;
    if !start.is_finite() || !end.is_finite() || start < 0.0 {
        return Err("That is not a valid range.".into());
    }
    if duration < MIN_SECONDS {
        return Err(format!(
            "That selection is {duration:.2} seconds long. Drag the handles further apart."
        ));
    }
    if !source.is_file() {
        return Err("That clip is not there any more.".into());
    }

    let target = output_path(source, replace);
    let scratch = source.with_file_name(format!(
        ".{}.trimming.mp4",
        source
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "clip".into())
    ));
    let _ = std::fs::remove_file(&scratch);

    // A copy has nothing to fall back to. Otherwise: the hardware encoder
    // already proven to work on this machine, then software. A GPU encoder
    // happy with the frames Desktop Duplication hands it can still refuse the
    // ones a file decode produces, and a trim that fails outright is worse
    // than one that takes longer.
    let mut attempts: Vec<Option<&str>> = Vec::new();
    if fast {
        attempts.push(None);
    } else {
        if let Some(p) = pipeline {
            attempts.push(Some(p.encoder));
        }
        if !attempts.contains(&Some("libx264")) {
            attempts.push(Some("libx264"));
        }
    }

    // A size limit overrides the capture bitrate: the point is landing under it,
    // not preserving the quality the recorder happened to be using. A stream
    // copy cannot change the bitrate it is copying, so `fast` wins.
    let mut settings = settings.clone();
    if let Some(bytes) = fit_bytes.filter(|_| !fast) {
        settings.bitrate_kbps = bitrate_to_fit(bytes, duration);
    }

    let mut last = String::new();
    for encoder in attempts {
        match run(
            ffmpeg_path,
            &settings,
            encoder,
            source,
            start,
            duration,
            &scratch,
        ) {
            Ok(()) => {
                let moved = std::fs::rename(&scratch, &target)
                    .map_err(|e| format!("could not save the trimmed clip: {e}"));
                if let Err(e) = moved {
                    let _ = std::fs::remove_file(&scratch);
                    return Err(e);
                }
                // Trimming a session produces an mp4 from an mkv, so replacing
                // leaves the untrimmed original sitting there under its old
                // extension unless it is removed on purpose.
                if replace && target != source {
                    let _ = std::fs::remove_file(source);
                }
                return Ok(target);
            }
            Err(e) => {
                let _ = std::fs::remove_file(&scratch);
                last = e;
            }
        }
    }
    Err(format!("Could not trim that clip. {last}"))
}

/// Where the trimmed clip lands.
///
/// Replacing writes over the source. Otherwise it takes a `_trimmed` name, and
/// a number after that, because trimming the same clip twice is a normal thing
/// to do and silently overwriting the first attempt is not.
fn output_path(source: &Path, replace: bool) -> PathBuf {
    if replace {
        return source.with_extension("mp4");
    }
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Clip".into());
    let mut candidate = source.with_file_name(format!("{stem}_trimmed.mp4"));
    let mut n = 2;
    while candidate.exists() {
        candidate = source.with_file_name(format!("{stem}_trimmed_{n}.mp4"));
        n += 1;
    }
    candidate
}

#[allow(clippy::too_many_arguments)]
fn run(
    ffmpeg_path: &Path,
    settings: &Settings,
    encoder: Option<&str>,
    source: &Path,
    start: f64,
    duration: f64,
    out: &Path,
) -> Result<(), String> {
    let output = ffmpeg::command(ffmpeg_path)
        .args(args(settings, encoder, source, start, duration, out))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("could not run ffmpeg: {e}"))?;

    if output.status.success() && out.is_file() {
        return Ok(());
    }
    Err(ffmpeg::explain(&String::from_utf8_lossy(&output.stderr)))
}

/// `-ss` before `-i` seeks by keyframe and then decodes forward to the exact
/// frame, which is both fast and accurate. `-t` after the input keeps the
/// duration relative to that seek rather than to the original timeline.
fn args(
    settings: &Settings,
    encoder: Option<&str>,
    source: &Path,
    start: f64,
    duration: f64,
    out: &Path,
) -> Vec<String> {
    let bitrate = format!("{}k", settings.bitrate_kbps);
    let mut a: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
        "-ss".into(),
        format!("{start:.3}"),
        "-i".into(),
        source.to_string_lossy().into_owned(),
        "-t".into(),
        format!("{duration:.3}"),
    ];

    match encoder {
        None => a.extend(["-c".into(), "copy".into()]),
        Some(encoder) => {
            a.extend(["-c:v".into(), encoder.into()]);
            // Quality-oriented rather than the capture path's constant bitrate:
            // a trim is not writing to a ring buffer, so there is no reason to
            // hold the file size predictable at the cost of the picture.
            match encoder {
                "h264_nvenc" => {
                    a.extend(["-preset".into(), "p5".into(), "-rc".into(), "vbr".into()])
                }
                "h264_amf" => a.extend(["-usage".into(), "transcoding".into()]),
                _ => a.extend(["-preset".into(), "medium".into()]),
            }
            a.extend([
                "-b:v".into(),
                bitrate.clone(),
                "-maxrate".into(),
                bitrate,
                "-pix_fmt".into(),
                "yuv420p".into(),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                "160k".into(),
            ]);
        }
    }

    // The trimmed clip is the one people upload, and an index at the front is
    // what lets a player start before the whole file has arrived.
    a.extend([
        "-movflags".into(),
        "+faststart".into(),
        out.to_string_lossy().into_owned(),
    ]);
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings {
            bitrate_kbps: 25_000,
            ..Default::default()
        }
    }

    #[test]
    fn seeks_before_the_input_and_limits_after_it() {
        let a = args(
            &settings(),
            Some("libx264"),
            Path::new("clip.mp4"),
            12.5,
            8.0,
            Path::new("out.mp4"),
        );
        let ss = a.iter().position(|x| x == "-ss").unwrap();
        let i = a.iter().position(|x| x == "-i").unwrap();
        let t = a.iter().position(|x| x == "-t").unwrap();
        // Order is the whole point: -ss after -i decodes the entire run-up, and
        // -t before -i would cut the wrong thing.
        assert!(ss < i, "-ss must come before -i");
        assert!(i < t, "-t must come after -i");
        assert_eq!(a[ss + 1], "12.500");
        assert_eq!(a[t + 1], "8.000");
    }

    #[test]
    fn a_copy_keeps_the_original() {
        let out = output_path(Path::new("/clips/Night.mp4"), false);
        assert_eq!(out, Path::new("/clips/Night_trimmed.mp4"));
    }

    #[test]
    fn replacing_writes_over_the_source() {
        let out = output_path(Path::new("/clips/Night.mp4"), true);
        assert_eq!(out, Path::new("/clips/Night.mp4"));
    }

    /// A session is Matroska; trimming one produces an mp4, and the name must
    /// not collide with the source it came from.
    #[test]
    fn a_trimmed_session_does_not_overwrite_its_source() {
        let out = output_path(Path::new("/sessions/Night.mkv"), false);
        assert_eq!(out, Path::new("/sessions/Night_trimmed.mp4"));
    }

    #[test]
    fn a_backwards_or_empty_range_is_refused() {
        let s = settings();
        let err = trim(
            Path::new("ffmpeg"),
            &s,
            None,
            Path::new("nope.mp4"),
            &Request {
                start: 10.0,
                end: 10.0,
                replace: false,
                fast: false,
                fit_bytes: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("Drag the handles"), "{err}");
    }
}

/// Exercises the real ffmpeg binary. Ignored by default because CI has no
/// ffmpeg on PATH; run with `FIVEMCLIP_TEST_FFMPEG=/path/to/ffmpeg`.
#[cfg(test)]
mod ffmpeg_tests {
    use super::*;

    fn ffmpeg_for_test() -> Option<PathBuf> {
        std::env::var_os("FIVEMCLIP_TEST_FFMPEG").map(PathBuf::from)
    }

    fn duration_of(ffmpeg: &Path, file: &Path) -> f64 {
        // ffprobe is not shipped, so ask ffmpeg to decode the file to nothing
        // and read the time it reports having processed.
        let out = ffmpeg::command(ffmpeg)
            .args([
                "-hide_banner",
                "-i",
                &file.to_string_lossy(),
                "-f",
                "null",
                "-",
            ])
            .output()
            .expect("ffmpeg runs");
        parse_time(&String::from_utf8_lossy(&out.stderr))
    }

    fn parse_time(stderr: &str) -> f64 {
        let marker = stderr
            .rmatch_indices("time=")
            .next()
            .map(|(i, _)| i)
            .expect("ffmpeg reports a time");
        let stamp = &stderr[marker + 5..marker + 16];
        let mut parts = stamp.split(':');
        let h: f64 = parts.next().unwrap().trim().parse().unwrap();
        let m: f64 = parts.next().unwrap().parse().unwrap();
        let s: f64 = parts.next().unwrap().parse().unwrap();
        h * 3600.0 + m * 60.0 + s
    }

    #[test]
    fn trims_to_the_requested_length() {
        let Some(ffmpeg) = ffmpeg_for_test() else {
            eprintln!("skipped: set FIVEMCLIP_TEST_FFMPEG");
            return;
        };
        let dir = std::env::temp_dir().join("fivemclip-trim-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("Clip.mp4");
        std::fs::copy(
            std::env::var("FIVEMCLIP_TEST_CLIP").expect("FIVEMCLIP_TEST_CLIP"),
            &source,
        )
        .unwrap();

        let settings = Settings {
            bitrate_kbps: 4_000,
            ..Default::default()
        };
        let out = trim(
            &ffmpeg,
            &settings,
            None,
            &source,
            &Request {
                start: 5.0,
                end: 12.0,
                replace: false,
                fast: false,
                fit_bytes: None,
            },
        )
        .expect("trim succeeds");

        assert_eq!(out.file_name().unwrap(), "Clip_trimmed.mp4");
        assert!(source.is_file(), "the original must survive a copy trim");

        let got = duration_of(&ffmpeg, &out);
        assert!(
            (got - 7.0).abs() < 0.35,
            "expected about 7s, got {got:.2}s - a keyframe-snapped cut would be out by up to two"
        );
    }

    #[test]
    fn replacing_leaves_one_file_behind() {
        let Some(ffmpeg) = ffmpeg_for_test() else {
            eprintln!("skipped: set FIVEMCLIP_TEST_FFMPEG");
            return;
        };
        let dir = std::env::temp_dir().join("fivemclip-trim-replace");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("Clip.mp4");
        std::fs::copy(
            std::env::var("FIVEMCLIP_TEST_CLIP").expect("FIVEMCLIP_TEST_CLIP"),
            &source,
        )
        .unwrap();

        let settings = Settings::default();
        let out = trim(
            &ffmpeg,
            &settings,
            None,
            &source,
            &Request {
                start: 2.0,
                end: 6.0,
                replace: true,
                fast: false,
                fit_bytes: None,
            },
        )
        .expect("trim succeeds");
        assert_eq!(out, source);

        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, vec!["Clip.mp4"], "no scratch file may be left behind");

        let got = duration_of(&ffmpeg, &source);
        assert!((got - 4.0).abs() < 0.35, "expected about 4s, got {got:.2}s");
    }

    /// Decode ignoring the MP4 edit list - what a player that does not honour
    /// one sees, which is the whole reason `fast` is not the default.
    fn raw_duration(ffmpeg: &Path, file: &Path) -> f64 {
        let out = ffmpeg::command(ffmpeg)
            .args([
                "-hide_banner",
                "-ignore_editlist",
                "1",
                "-i",
                &file.to_string_lossy(),
                "-f",
                "null",
                "-",
            ])
            .output()
            .expect("ffmpeg runs");
        parse_time(&String::from_utf8_lossy(&out.stderr))
    }

    /// The claim the default rests on: a fast trim physically keeps footage the
    /// user asked to remove, and an exact one does not.
    ///
    /// Measured rather than assumed - the first version of this module
    /// re-encoded on the belief that a copy trim also *plays* from the wrong
    /// place, which turned out to be false. It plays from the right one and
    /// hides the rest.
    #[test]
    fn a_fast_trim_keeps_what_it_appears_to_cut() {
        let Some(ffmpeg) = ffmpeg_for_test() else {
            eprintln!("skipped: set FIVEMCLIP_TEST_FFMPEG");
            return;
        };
        let dir = std::env::temp_dir().join("fivemclip-trim-fast");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let clip = std::env::var("FIVEMCLIP_TEST_CLIP").expect("FIVEMCLIP_TEST_CLIP");

        let settings = Settings {
            bitrate_kbps: 4_000,
            ..Default::default()
        };

        // 5.0 sits between keyframes, which are two seconds apart.
        let fast_src = dir.join("Fast.mp4");
        std::fs::copy(&clip, &fast_src).unwrap();
        let fast = trim(
            &ffmpeg,
            &settings,
            None,
            &fast_src,
            &Request {
                start: 5.0,
                end: 12.0,
                replace: false,
                fast: true,
                fit_bytes: None,
            },
        )
        .unwrap();

        let exact_src = dir.join("Exact.mp4");
        std::fs::copy(&clip, &exact_src).unwrap();
        let exact = trim(
            &ffmpeg,
            &settings,
            None,
            &exact_src,
            &Request {
                start: 5.0,
                end: 12.0,
                replace: false,
                fast: false,
                fit_bytes: None,
            },
        )
        .unwrap();

        // Both play back as the seven seconds that were asked for.
        assert!((duration_of(&ffmpeg, &fast) - 7.0).abs() < 0.35);
        assert!((duration_of(&ffmpeg, &exact) - 7.0).abs() < 0.35);

        // Only one of them is actually seven seconds of data.
        let fast_raw = raw_duration(&ffmpeg, &fast);
        let exact_raw = raw_duration(&ffmpeg, &exact);
        assert!(
            fast_raw > 7.4,
            "a fast trim should still contain the run-up, got {fast_raw:.2}s"
        );
        assert!(
            exact_raw < 7.35,
            "an exact trim must contain nothing but the selection, got {exact_raw:.2}s"
        );
    }

    /// The scratch file must not survive a failure, and the original must be
    /// exactly as it was.
    #[test]
    fn a_failed_trim_leaves_the_original_alone() {
        let Some(ffmpeg) = ffmpeg_for_test() else {
            eprintln!("skipped: set FIVEMCLIP_TEST_FFMPEG");
            return;
        };
        let dir = std::env::temp_dir().join("fivemclip-trim-fail");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("Clip.mp4");
        std::fs::write(&source, b"this is not a video").unwrap();

        let settings = Settings::default();
        let err = trim(
            &ffmpeg,
            &settings,
            None,
            &source,
            &Request {
                start: 0.0,
                end: 5.0,
                replace: true,
                fast: false,
                fit_bytes: None,
            },
        )
        .unwrap_err();
        assert!(!err.is_empty());
        assert_eq!(
            std::fs::read(&source).unwrap(),
            b"this is not a video",
            "the original must be untouched"
        );
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, vec!["Clip.mp4"], "no scratch file may be left behind");
    }
}

#[cfg(test)]
mod fit_tests {
    use super::*;

    /// The trimmer shows the bitrate its selection will be encoded at. If the
    /// front end's arithmetic and this one disagree, the readout is a lie about
    /// the file the user is about to get.
    ///
    /// These are the numbers `ui/trim.js` produced for a 25 MB limit, read
    /// straight out of the running page.
    #[test]
    fn matches_what_the_front_end_shows() {
        for (seconds, expected_kbps) in
            [(5.0, 37_840), (10.0, 18_840), (15.0, 12_507), (20.0, 9_340)]
        {
            let got = bitrate_to_fit(25_000_000, seconds);
            assert!(
                got.abs_diff(expected_kbps) <= 1,
                "{seconds}s: front end says {expected_kbps} kbps, backend says {got}"
            );
        }
    }

    /// Leaving headroom is the point: landing at 100.4% of a hard cap means the
    /// upload is refused and the whole re-encode was wasted.
    #[test]
    fn the_estimate_stays_under_the_limit() {
        for seconds in [3.0, 12.0, 60.0, 240.0] {
            let kbps = bitrate_to_fit(25_000_000, seconds);
            let predicted_bytes = ((kbps + AUDIO_KBPS) as f64 * 1000.0 * seconds / 8.0) as u64;
            assert!(
                predicted_bytes < 25_000_000,
                "{seconds}s would land at {predicted_bytes} bytes"
            );
        }
    }

    /// A clip long enough that fitting it would smear it is not worth sending,
    /// and the app says so rather than producing one.
    #[test]
    fn a_long_clip_falls_below_what_is_worth_watching() {
        // Five minutes into 25 MB.
        assert!(bitrate_to_fit(25_000_000, 300.0) < MIN_USEFUL_KBPS);
        // Thirty seconds into 25 MB is comfortably fine.
        assert!(bitrate_to_fit(25_000_000, 30.0) > MIN_USEFUL_KBPS);
    }

    #[test]
    fn a_zero_length_selection_does_not_divide_by_zero() {
        assert_eq!(bitrate_to_fit(25_000_000, 0.0), MIN_USEFUL_KBPS);
    }
}
