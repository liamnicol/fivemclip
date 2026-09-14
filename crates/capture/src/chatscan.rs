//! Finding the chat lines worth hiding, and when they are on screen.
//!
//! The obvious approach is colour, and it does not work. Every faction picks
//! its own chat colour, so two faction lines can be green and blue while an
//! unrelated channel matches either. Measured on real footage: hue separates
//! the *tags* cleanly - `[INFO]` at 60°, `[RADIO]` 38°, `[DISPATCH]` 210° - but
//! not the *channels*, which is the thing being asked for. Brightness is worse
//! still; the fraction of bright pixels in the chat region inverts between a
//! daylight scene and a night one.
//!
//! What is constant is the word in the tag. So the lines are read.
//!
//! Nothing here applies anything on its own. It proposes rectangles, the
//! trimmer shows them, and a person confirms - because a detector that
//! silently misses one line publishes the thing it was asked to hide, which is
//! the same trap as a blur that leaves text legible.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::config::ChatRegion;
use crate::ffmpeg;
use crate::trim::Blackout;
use rten_imageproc::BoundingRect;

/// How often to look, in seconds.
///
/// Chat is readable for seconds at a time, so a twice-a-second glance cannot
/// miss a line that a human could have read. Lower would cost OCR passes for
/// nothing; higher risks a message that arrives and scrolls away between looks.
pub const SAMPLE_SECONDS: f64 = 0.5;

/// One line of chat, as read off one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// Seconds from the start of the video.
    pub at: f64,
    /// Where it sits, as fractions of the whole frame.
    pub region: ChatRegion,
    pub text: String,
}

/// Whether `text` looks like it belongs to a channel the user named.
///
/// Deliberately loose. OCR of game text returns `fPaction System` for
/// `[Faction System]` and `ADVERTlSING` for `ADVERTISING` - single characters,
/// every time - so matching exactly would miss the lines that matter while
/// looking like it worked.
pub fn mentions(text: &str, keyword: &str) -> bool {
    let keyword = keyword.trim().to_ascii_lowercase();
    if keyword.is_empty() {
        return false;
    }
    let text = text.to_ascii_lowercase();
    let head = tag_of(&text);
    head.contains(&keyword) || near_match(&head, &keyword)
}

/// The part of a line that can be the channel tag.
///
/// The first few words, and not "up to the closing bracket" - OCR loses the
/// brackets. `[Faction System]` came back as `fPaction System` off real
/// footage, both brackets gone, so anything keyed on them would miss exactly
/// the lines that matter while appearing to work.
///
/// Three words is enough for every tag seen - `[Faction | Reserve Correctional
/// Officer]` puts the word first, and `** [RADIO] Pax:` puts it second - and
/// short enough that someone saying "my faction is recruiting" in ordinary chat
/// is not mistaken for the channel. Hiding their sentence would be both wrong
/// and baffling.
fn tag_of(text: &str) -> String {
    text.split_whitespace()
        .take(TAG_WORDS)
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(TAG_CHARS)
        .collect()
}

const TAG_WORDS: usize = 3;
const TAG_CHARS: usize = 64;

/// Is `keyword` in `head` give or take one character per five?
fn near_match(head: &str, keyword: &str) -> bool {
    let k: Vec<char> = keyword.chars().collect();
    let h: Vec<char> = head.chars().collect();
    if k.len() > h.len() {
        return false;
    }
    // A fifth was not enough against real footage: `[Faction System]` over a
    // bright window came back as `ofP?ction System`, two characters wrong in
    // seven, and was missed - which for a redactor means the line is published.
    // A third still refuses every other tag in the same frame.
    let allowed = (k.len() / 3).max(1);
    // Every window the keyword could sit in. Short strings, so the cost of
    // being obvious here is nothing.
    h.windows(k.len())
        .any(|w| w.iter().zip(&k).filter(|(a, b)| a != b).count() <= allowed)
}

/// Turn lines read off frames into rectangles to paint out.
///
/// `start` is where the trim begins, because a filter's timestamps are relative
/// to the trimmed output rather than to the source.
///
/// A line with no tag of its own inherits the channel of the line above it:
/// a wrapped message continues in plain white, and hiding only its first line
/// would leave the rest of it readable - which is worse than not hiding it at
/// all, because it looks handled.
///
/// Known limitation, seen on real footage and not yet solved: an untagged line
/// that is a *new* message rather than a continuation inherits too. A `/me`
/// renders as `* Pax_Amicolli grabs the body camera`, no tag, and directly
/// under a matched `[DISPATCH]` line it gets covered along with it. Erring this
/// way is the right way to err - covering one line too many beats publishing
/// one too few - but it is why what comes out of here is shown for correction
/// rather than applied.
pub fn blackouts(lines: &[Line], keywords: &[String], start: f64) -> Vec<Blackout> {
    let mut out = Vec::new();
    for group in group_by_frame(lines) {
        let mut hiding = false;
        for line in group {
            let tagged = line.text.trim_start().starts_with('[');
            if tagged {
                hiding = keywords.iter().any(|k| mentions(&line.text, k));
            }
            // An untagged line keeps whatever the line above decided.
            if !hiding {
                continue;
            }
            out.push(Blackout {
                region: line.region,
                from: (line.at - start - SAMPLE_SECONDS / 2.0).max(0.0),
                to: line.at - start + SAMPLE_SECONDS / 2.0,
            });
        }
    }
    merge(out)
}

/// Lines from one frame, in the order they appear down the screen.
fn group_by_frame(lines: &[Line]) -> Vec<Vec<&Line>> {
    let mut frames: Vec<(f64, Vec<&Line>)> = Vec::new();
    for line in lines {
        match frames
            .iter_mut()
            .find(|(at, _)| (*at - line.at).abs() < 1e-6)
        {
            Some((_, group)) => group.push(line),
            None => frames.push((line.at, vec![line])),
        }
    }
    frames.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, group) in &mut frames {
        group.sort_by(|a, b| a.region.y.total_cmp(&b.region.y));
    }
    frames.into_iter().map(|(_, g)| g).collect()
}

/// Join boxes that are the same rectangle in consecutive samples.
///
/// Without this a ten second message is twenty identical drawbox filters, each
/// gated on half a second - which works, and makes a filter graph nobody can
/// read and ffmpeg has to parse.
fn merge(mut boxes: Vec<Blackout>) -> Vec<Blackout> {
    boxes.sort_by(|a, b| {
        a.region
            .y
            .total_cmp(&b.region.y)
            .then(a.from.total_cmp(&b.from))
    });
    let mut out: Vec<Blackout> = Vec::new();
    for b in boxes {
        match out.last_mut() {
            // Same rectangle, and the next sample along: extend rather than add.
            Some(last)
                if same_rect(&last.region, &b.region)
                    && b.from <= last.to + SAMPLE_SECONDS / 2.0 =>
            {
                last.to = last.to.max(b.to);
            }
            _ => out.push(b),
        }
    }
    out
}

fn same_rect(a: &ChatRegion, b: &ChatRegion) -> bool {
    let close = |x: f32, y: f32| (x - y).abs() < 0.004;
    close(a.x, b.x) && close(a.y, b.y) && close(a.w, b.w) && close(a.h, b.h)
}

/// Pull one frame of `region` out of `video` as a PNG.
pub fn frame_at(
    ffmpeg_path: &Path,
    video: &Path,
    region: &ChatRegion,
    at: f64,
    out: &Path,
) -> Result<(), String> {
    let crop = format!(
        "crop=in_w*{:.4}:in_h*{:.4}:in_w*{:.4}:in_h*{:.4},scale=iw*2:ih*2:flags=lanczos",
        region.w, region.h, region.x, region.y
    );
    let output = ffmpeg::command(ffmpeg_path)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-ss",
            &format!("{at:.3}"),
            "-i",
            &video.to_string_lossy(),
            "-frames:v",
            "1",
            "-vf",
            &crop,
            &out.to_string_lossy(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| ffmpeg::spawn_error(ffmpeg_path, &e))?;

    if output.status.success() && out.is_file() {
        return Ok(());
    }
    Err(ffmpeg::explain(&String::from_utf8_lossy(&output.stderr)))
}

/// Where the OCR models live.
#[derive(Debug, Clone)]
pub struct Models {
    pub detection: PathBuf,
    pub recognition: PathBuf,
}

impl Models {
    /// Beside the executable, the way ffmpeg is found.
    pub fn beside_exe() -> Option<Self> {
        let exe = std::env::current_exe().ok()?;
        let dir = exe.parent()?;
        for base in [dir.join("models"), dir.to_path_buf()] {
            let detection = base.join("text-detection.rten");
            let recognition = base.join("text-recognition.rten");
            if detection.is_file() && recognition.is_file() {
                return Some(Self {
                    detection,
                    recognition,
                });
            }
        }
        None
    }
}

/// The one rectangle that contains every word of a line.
fn bounds(line: &[rten_imageproc::RotatedRect]) -> Option<(f32, f32, f32, f32)> {
    line.iter().fold(None, |acc, w| {
        let r = w.bounding_rect();
        let (l, t, rt, bt) = (r.left(), r.top(), r.right(), r.bottom());
        Some(match acc {
            None => (l, t, rt, bt),
            Some((al, at, ar, ab)) => (al.min(l), at.min(t), ar.max(rt), ab.max(bt)),
        })
    })
}

/// Read every chat line in `video` between `start` and `end`.
///
/// Samples rather than reads every frame: OCR costs about a second a frame, and
/// chat stays on screen for seconds, so a twice-a-second glance cannot miss a
/// line a person could have read.
pub fn scan(
    ffmpeg_path: &Path,
    models: &Models,
    video: &Path,
    region: &ChatRegion,
    start: f64,
    end: f64,
) -> Result<Vec<Line>, String> {
    use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
    use rten::Model;

    let detection = Model::load_file(&models.detection)
        .map_err(|e| format!("could not load the text detector: {e}"))?;
    let recognition = Model::load_file(&models.recognition)
        .map_err(|e| format!("could not load the text reader: {e}"))?;
    let engine = OcrEngine::new(OcrEngineParams {
        detection_model: Some(detection),
        recognition_model: Some(recognition),
        ..Default::default()
    })
    .map_err(|e| format!("could not start the text reader: {e}"))?;

    let work = std::env::temp_dir().join(format!("fivemclip-scan-{}", std::process::id()));
    std::fs::create_dir_all(&work).map_err(|e| format!("could not make a scratch folder: {e}"))?;
    let shot = work.join("frame.png");

    let mut lines = Vec::new();
    let mut at = start;
    while at < end {
        frame_at(ffmpeg_path, video, region, at, &shot)?;
        let image = image::open(&shot)
            .map_err(|e| format!("could not read the frame back: {e}"))?
            .into_rgb8();
        let source = ImageSource::from_bytes(image.as_raw(), image.dimensions())
            .map_err(|e| format!("could not hand the frame to the reader: {e}"))?;
        let input = engine
            .prepare_input(source)
            .map_err(|e| format!("could not prepare the frame: {e}"))?;
        let words = engine
            .detect_words(&input)
            .map_err(|e| format!("could not find the text: {e}"))?;
        let found = engine.find_text_lines(&input, &words);
        let texts = engine
            .recognize_text(&input, &found)
            .map_err(|e| format!("could not read the text: {e}"))?;

        // The crop was scaled up for the reader's benefit, and it is expressed
        // as a fraction of the crop - both have to come back out before the
        // rectangle means anything against the whole frame.
        let (cw, ch) = (image.width() as f32, image.height() as f32);
        for (line, text) in found.iter().zip(texts.iter()) {
            let Some(text) = text else { continue };
            let text = text.to_string();
            if text.trim().is_empty() {
                continue;
            }
            let Some((l, t, r, b)) = bounds(line) else {
                continue;
            };
            lines.push(Line {
                at,
                region: ChatRegion {
                    x: region.x + region.w * (l / cw),
                    y: region.y + region.h * (t / ch),
                    w: region.w * ((r - l) / cw),
                    h: region.h * ((b - t) / ch),
                },
                text,
            });
        }
        at += SAMPLE_SECONDS;
    }

    let _ = std::fs::remove_dir_all(&work);
    Ok(lines)
}

#[cfg(test)]
mod real_footage_tests {
    use super::*;

    /// End to end on a real recording, because everything upstream of this was
    /// checked against footage I invented and the invented version was wrong
    /// twice - first about the chat having a dark backing panel, then about
    /// colour telling the channels apart.
    ///
    /// Needs a clip and the models; skips without them.
    #[test]
    fn it_reads_the_chat_off_a_real_clip() {
        let (Ok(ffmpeg), Ok(clip), Ok(models)) = (
            std::env::var("FIVEMCLIP_TEST_FFMPEG"),
            std::env::var("FIVEMCLIP_TEST_CHAT_CLIP"),
            std::env::var("FIVEMCLIP_TEST_MODELS"),
        ) else {
            eprintln!("skipped: set FIVEMCLIP_TEST_FFMPEG, _CHAT_CLIP and _MODELS");
            return;
        };
        let models = Models {
            detection: PathBuf::from(&models).join("text-detection.rten"),
            recognition: PathBuf::from(&models).join("text-recognition.rten"),
        };
        // The chat sits in the top left. Measured off the clip.
        let region = ChatRegion {
            x: 0.015,
            y: 0.018,
            w: 0.43,
            h: 0.30,
        };

        let lines = scan(
            Path::new(&ffmpeg),
            &models,
            Path::new(&clip),
            &region,
            8.5,
            9.5,
        )
        .expect("the scan runs");

        assert!(!lines.is_empty(), "nothing was read at all");
        for l in &lines {
            eprintln!("  {:.1}s y={:.3} {}", l.at, l.region.y, l.text);
        }

        let all: String = lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(all.to_lowercase().contains("dispatch"), "{all}");
        assert!(all.to_lowercase().contains("income"), "{all}");

        // Every box has to land inside the region it was cropped from, or the
        // blackout covers the wrong part of the picture.
        for l in &lines {
            assert!(l.region.x >= region.x - 0.001, "{l:?}");
            assert!(l.region.y >= region.y - 0.001, "{l:?}");
            assert!(
                l.region.x + l.region.w <= region.x + region.w + 0.001,
                "{l:?}"
            );
            assert!(
                l.region.y + l.region.h <= region.y + region.h + 0.001,
                "{l:?}"
            );
        }

        // And the thing the whole feature is for.
        let hidden = blackouts(&lines, &["faction".into(), "dispatch".into()], 0.0);
        assert!(!hidden.is_empty(), "nothing matched");
        eprintln!("  -> {} blackout(s)", hidden.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(at: f64, y: f32, text: &str) -> Line {
        Line {
            at,
            region: ChatRegion {
                x: 0.02,
                y,
                w: 0.4,
                h: 0.025,
            },
            text: text.into(),
        }
    }

    /// What the OCR actually returned on real footage. Exact matching would
    /// have missed every one of these.
    #[test]
    fn a_misread_tag_still_matches() {
        assert!(mentions(
            "[fPaction System] You have set your duty mode",
            "faction"
        ));
        assert!(mentions(
            "[WEAZEL ADVERTlSING]: Frenchie's is Now Open",
            "weazel"
        ));
        assert!(mentions(
            "[RADlO] Pax_Amicolli: 30675 starting shift",
            "radio"
        ));
        assert!(mentions("[DISPATCH] *** Unit with callsign", "dispatch"));
    }

    /// The worst real reading seen, from the tag sitting over a bright window.
    #[test]
    fn the_worst_real_misreading_still_matches() {
        assert!(mentions(
            "ofP?ction System You have set your duty mode on!",
            "faction"
        ));
    }

    /// The same frame's other tags, against the same loosened tolerance. If any
    /// of these start matching, the tolerance has gone too far and unrelated
    /// chat gets blacked out.
    #[test]
    fn loosening_the_match_does_not_catch_the_neighbours() {
        for line in [
            "[INFO]: You have received base income of $100 from the government!",
            "[WEAZEL ADVERTISINGJ Frenchie's is Now Open Los Santos' finest",
            "* IRADIO] Pax Amicolli: 30675 starting shift under 5-AGENT-10",
            "[DISPATCH]  Unit with callsign '5-AGENT-10' created by Pax",
            "restaurant High Quality Food & Handcrafted Beverages 8 Elgin Ave",
            "hitting recoro",
        ] {
            assert!(!mentions(line, "faction"), "{line}");
        }
    }

    #[test]
    fn an_unrelated_channel_does_not_match() {
        assert!(!mentions(
            "[INFO]: You have received base income",
            "faction"
        ));
        assert!(!mentions("[DISPATCH] *** Unit with callsign", "faction"));
    }

    /// Saying the word is not being the channel. Hiding someone's sentence
    /// because it mentions their faction would be wrong and baffling.
    #[test]
    fn the_word_in_the_body_of_a_message_is_not_a_tag() {
        let said = "Pax_Amicolli says: my faction is recruiting, ask me about it \
                    if you want to join the faction today";
        assert!(!mentions(said, "faction"));
    }

    /// The tag is not always the first word. A radio line off real footage
    /// starts with a pair of asterisks.
    #[test]
    fn a_tag_a_word_or_two_in_still_counts() {
        assert!(mentions(
            "** [RADIO] Pax_Amicolli: 30675 starting shift",
            "radio"
        ));
        assert!(mentions(
            "* [Faction | Security Senior Agent] Pax: test",
            "faction"
        ));
    }

    /// The long tags are the ones a faction writes itself, and they are where
    /// a character budget would quietly start failing.
    #[test]
    fn a_long_faction_tag_is_still_read_as_one() {
        assert!(mentions(
            "[Faction | Reserve Correctional Officer] Pax_Amicolli: ((( test )))",
            "faction"
        ));
    }

    #[test]
    fn an_empty_keyword_matches_nothing() {
        assert!(!mentions("[Faction] anything", ""));
        assert!(!mentions("[Faction] anything", "   "));
    }

    /// A wrapped message keeps going in plain white with no tag. Hiding only
    /// its first line leaves the rest readable while looking handled.
    #[test]
    fn a_wrapped_line_inherits_the_channel_above_it() {
        let lines = vec![
            line(1.0, 0.10, "[Faction | Security] Pax: the first half of it"),
            line(1.0, 0.13, "and the second half of it"),
            line(1.0, 0.16, "[INFO]: something else entirely"),
        ];
        let out = blackouts(&lines, &["faction".into()], 0.0);
        assert_eq!(out.len(), 2, "{out:?}");
        assert!((out[0].region.y - 0.10).abs() < 1e-6);
        assert!(
            (out[1].region.y - 0.13).abs() < 1e-6,
            "the wrap must be covered"
        );
    }

    #[test]
    fn a_tagged_line_ends_the_inheritance() {
        let lines = vec![
            line(1.0, 0.10, "[Faction] hide me"),
            line(1.0, 0.13, "[INFO]: leave me alone"),
            line(1.0, 0.16, "my continuation"),
        ];
        let out = blackouts(&lines, &["faction".into()], 0.0);
        assert_eq!(out.len(), 1, "{out:?}");
    }

    /// The chat scrolls, so the same message is a different rectangle later.
    #[test]
    fn a_line_that_moves_gets_a_box_in_each_place() {
        let lines = vec![
            line(1.0, 0.20, "[Faction] same message"),
            line(1.5, 0.17, "[Faction] same message"),
        ];
        let out = blackouts(&lines, &["faction".into()], 0.0);
        assert_eq!(out.len(), 2, "{out:?}");
    }

    #[test]
    fn a_line_that_stays_put_is_one_box_not_twenty() {
        let lines: Vec<Line> = (0..20)
            .map(|i| {
                line(
                    1.0 + i as f64 * SAMPLE_SECONDS,
                    0.20,
                    "[Faction] still here",
                )
            })
            .collect();
        let out = blackouts(&lines, &["faction".into()], 0.0);
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(out[0].to - out[0].from > 9.0, "{out:?}");
    }

    /// Filter timestamps are relative to the trimmed output, so a scan of the
    /// source has to have the trim's start taken off it.
    #[test]
    fn times_are_shifted_by_where_the_trim_begins() {
        let lines = vec![line(12.0, 0.20, "[Faction] hide me")];
        let out = blackouts(&lines, &["faction".into()], 10.0);
        assert_eq!(out.len(), 1);
        assert!((out[0].from - 1.75).abs() < 1e-6, "{out:?}");
    }

    #[test]
    fn nothing_matching_hides_nothing() {
        let lines = vec![line(1.0, 0.10, "[INFO]: nothing to see")];
        assert!(blackouts(&lines, &["faction".into()], 0.0).is_empty());
    }
}
