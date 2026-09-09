//! The replay buffer.
//!
//! ffmpeg writes a continuous stream of short MPEG-TS segments into a ring
//! directory and recycles the filenames. Saving a clip is then just picking the
//! newest N segments and concatenating them without re-encoding, which is why
//! pressing the hotkey feels instant even for a five minute buffer.
//!
//! MPEG-TS rather than MP4 is deliberate: TS survives being read while it is
//! still being written, so the few seconds that matter most - the ones still in
//! flight when the user hits the key - are recoverable.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use crate::audio::{self, Mixer};
use crate::config::{MicMode, Settings};
use crate::ffmpeg::{self, Pipeline};

/// Shorter segments mean a tighter clip boundary but more files. Two seconds
/// caps the overshoot at the head of a clip at two seconds, which nobody
/// notices, while keeping a 30 minute buffer under a thousand files.
pub const SEGMENT_SECONDS: u32 = 2;

pub struct Recorder {
    ffmpeg: PathBuf,
    settings: Settings,
    pipeline: &'static Pipeline,
    running: Option<Running>,
}

struct Running {
    child: Child,
    stop: Arc<AtomicBool>,
    audio_thread: Option<JoinHandle<()>>,
    started: Instant,
    /// Non-fatal problems worth surfacing, e.g. "recording without audio".
    warnings: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RecorderStatus {
    pub running: bool,
    pub seconds_buffered: u32,
    pub pipeline: String,
    pub has_audio: bool,
    pub warnings: Vec<String>,
}

impl Recorder {
    pub fn new(ffmpeg: PathBuf, settings: Settings, pipeline: &'static Pipeline) -> Self {
        Self {
            ffmpeg,
            settings,
            pipeline,
            running: None,
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn is_running(&mut self) -> bool {
        // Reap a crashed ffmpeg so the UI does not keep claiming to record.
        if let Some(r) = &mut self.running {
            if matches!(r.child.try_wait(), Ok(Some(_))) {
                self.running = None;
            }
        }
        self.running.is_some()
    }

    pub fn status(&mut self) -> RecorderStatus {
        let running = self.is_running();
        let (seconds, warnings, has_audio) = match &self.running {
            Some(r) => (
                (r.started.elapsed().as_secs() as u32).min(self.settings.buffer_seconds),
                r.warnings.clone(),
                r.audio_thread.is_some(),
            ),
            None => (0, Vec::new(), false),
        };
        RecorderStatus {
            running,
            seconds_buffered: seconds,
            pipeline: self.pipeline.label.to_string(),
            has_audio,
            warnings,
        }
    }

    pub fn apply_settings(&mut self, settings: Settings, pipeline: &'static Pipeline) {
        let restart = self.is_running();
        if restart {
            self.stop();
        }
        self.settings = settings;
        self.pipeline = pipeline;
        if restart {
            let _ = self.start();
        }
    }

    pub fn start(&mut self) -> Result<RecorderStatus, String> {
        if self.is_running() {
            return Ok(self.status());
        }

        let ring = self.settings.ring_dir();
        fs::create_dir_all(&ring).map_err(|e| format!("could not create buffer folder: {e}"))?;
        clear_ring(&ring);

        let mut warnings = Vec::new();

        // Open audio before ffmpeg so a failure here downgrades the ffmpeg
        // command line to video-only rather than leaving a dangling process.
        let system = match audio::start(audio::Source::System) {
            Ok(c) => Some(c),
            Err(e) => {
                warnings.push(format!("Recording without game audio: {e}"));
                None
            }
        };
        let mic = if self.settings.mic_mode == MicMode::Mixed && system.is_some() {
            match audio::start(audio::Source::Microphone) {
                Ok(c) => Some(c),
                Err(e) => {
                    warnings.push(format!("Recording without microphone: {e}"));
                    None
                }
            }
        } else {
            None
        };

        let has_audio = system.is_some();
        let args = self.build_args(has_audio, &ring);

        let mut cmd = ffmpeg::command(&self.ffmpeg);
        cmd.args(&args).stdout(Stdio::null()).stderr(Stdio::null());
        if has_audio {
            cmd.stdin(Stdio::piped());
        } else {
            cmd.stdin(Stdio::null());
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("could not start ffmpeg: {e}"))?;

        let stop = Arc::new(AtomicBool::new(false));
        let audio_thread = match (system, child.stdin.take()) {
            (Some(sys), Some(mut stdin)) => {
                let stop_thread = stop.clone();
                let mixer = Mixer {
                    system_gain: audio::db_to_linear(self.settings.system_gain_db),
                    mic_gain: audio::db_to_linear(self.settings.mic_gain_db),
                };
                let sys_rx = sys;
                let mic_rx = mic;
                Some(
                    std::thread::Builder::new()
                        .name("audio-mixer".into())
                        .spawn(move || {
                            // Keep the Capture handles alive for the whole mix;
                            // dropping them signals their threads to stop.
                            let microphone_rx = mic_rx.as_ref().map(|m| &m.rx);
                            mixer.run(&sys_rx.rx, microphone_rx, &mut stdin, &stop_thread);
                        })
                        .map_err(|e| format!("could not start audio mixer: {e}"))?,
                )
            }
            _ => None,
        };

        self.running = Some(Running {
            child,
            stop,
            audio_thread,
            started: Instant::now(),
            warnings,
        });
        Ok(self.status())
    }

    pub fn stop(&mut self) {
        if let Some(mut r) = self.running.take() {
            r.stop.store(true, Ordering::Relaxed);
            // Killing ffmpeg leaves the current segment truncated, which TS
            // tolerates. There is nothing to finalise because we never opened an
            // MP4 container on the hot path.
            let _ = r.child.kill();
            let _ = r.child.wait();
            if let Some(t) = r.audio_thread {
                let _ = t.join();
            }
        }
    }

    fn build_args(&self, has_audio: bool, ring: &Path) -> Vec<String> {
        let s = &self.settings;
        let mut a: Vec<String> = vec![
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-thread_queue_size".into(),
            "512".into(),
        ];
        a.extend(self.pipeline.video_input_args(s));

        if has_audio {
            a.extend([
                "-thread_queue_size".into(),
                "512".into(),
                "-f".into(),
                "f32le".into(),
                "-ar".into(),
                audio::SAMPLE_RATE.to_string(),
                "-ac".into(),
                audio::CHANNELS.to_string(),
                "-i".into(),
                "pipe:0".into(),
            ]);
        }

        a.extend([
            "-filter_complex".into(),
            format!("[0:v]{}[v]", self.pipeline.video_filter()),
            "-map".into(),
            "[v]".into(),
        ]);
        if has_audio {
            a.extend([
                "-map".into(),
                "1:a".into(),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                "192k".into(),
            ]);
        }
        a.extend(self.pipeline.encoder_args(s));

        // Enough slots for the whole buffer plus a couple in hand, so the
        // segment currently being written never clobbers one we still need.
        let wrap = s.buffer_seconds.div_ceil(SEGMENT_SECONDS) + 2;
        a.extend([
            "-f".into(),
            "segment".into(),
            "-segment_time".into(),
            SEGMENT_SECONDS.to_string(),
            "-segment_wrap".into(),
            wrap.to_string(),
            "-segment_format".into(),
            "mpegts".into(),
            // Without resent headers a segment that happens to be the first one
            // we concatenate has no decoder configuration and the clip opens black.
            "-segment_format_options".into(),
            "mpegts_flags=+resend_headers".into(),
            "-reset_timestamps".into(),
            "1".into(),
            ring.join("seg%05d.ts").to_string_lossy().into_owned(),
        ]);
        a
    }

    /// Concatenate the newest `seconds` of buffer into an MP4.
    pub fn save_clip(&mut self, seconds: u32) -> Result<PathBuf, String> {
        if !self.is_running() {
            return Err("The replay buffer is not running.".into());
        }
        let s = &self.settings;
        let seconds = seconds.clamp(SEGMENT_SECONDS, s.buffer_seconds);
        let ring = s.ring_dir();

        let mut segments = newest_segments(&ring, seconds)?;
        if segments.is_empty() {
            return Err("Nothing buffered yet - give it a few seconds.".into());
        }
        segments.sort_by_key(|(_, m)| *m);

        let clips = s.clips_dir();
        fs::create_dir_all(&clips).map_err(|e| format!("could not create clips folder: {e}"))?;

        let list_path = ring.join("concat.txt");
        let mut list = String::new();
        for (path, _) in &segments {
            // The concat demuxer treats backslashes as escapes, and single
            // quotes must be broken out of.
            let p = path
                .to_string_lossy()
                .replace('\\', "/")
                .replace('\'', "'\\''");
            list.push_str(&format!("file '{p}'\n"));
        }
        fs::write(&list_path, list).map_err(|e| format!("could not stage clip: {e}"))?;

        let out = clips.join(format!(
            "Clip_{}.mp4",
            chrono::Local::now().format("%Y-%m-%d_%H-%M-%S")
        ));

        let status = ffmpeg::command(&self.ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-y",
                "-fflags",
                "+genpts",
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
            ])
            .arg(&list_path)
            .args([
                "-c",
                "copy",
                // AAC carried in TS uses ADTS framing, which MP4 will not accept.
                "-bsf:a",
                "aac_adtstoasc",
                "-movflags",
                "+faststart",
            ])
            .arg(&out)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| format!("could not run ffmpeg: {e}"))?;

        let _ = fs::remove_file(&list_path);

        if !status.status.success() {
            let err = String::from_utf8_lossy(&status.stderr);
            return Err(format!(
                "Could not save the clip: {}",
                err.lines().last().unwrap_or("unknown ffmpeg error").trim()
            ));
        }
        Ok(out)
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop();
    }
}

fn clear_ring(ring: &Path) {
    if let Ok(entries) = fs::read_dir(ring) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "ts").unwrap_or(false)
                || p.file_name().map(|n| n == "concat.txt").unwrap_or(false)
            {
                let _ = fs::remove_file(p);
            }
        }
    }
}

/// Newest segments covering at least `seconds`, plus one extra so the clip is
/// never short at the head after the boundary lands mid-segment.
fn newest_segments(
    ring: &Path,
    seconds: u32,
) -> Result<Vec<(PathBuf, std::time::SystemTime)>, String> {
    let want = (seconds.div_ceil(SEGMENT_SECONDS) + 1) as usize;

    let mut files: Vec<(PathBuf, std::time::SystemTime)> = fs::read_dir(ring)
        .map_err(|e| format!("could not read the buffer folder: {e}"))?
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .map(|x| x.eq_ignore_ascii_case("ts"))
                .unwrap_or(false)
        })
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            // Skip empty files: ffmpeg creates the next segment before it has
            // written anything to it.
            if m.len() == 0 {
                return None;
            }
            Some((e.path(), m.modified().ok()?))
        })
        .collect();

    files.sort_by_key(|(_, m)| std::cmp::Reverse(*m));
    files.truncate(want);
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Writes segments oldest-first. NTFS timestamps are coarse, so the pause
    /// has to be generous enough that the ordering is unambiguous everywhere.
    fn seed(dir: &Path, names: &[(&str, usize)]) {
        for (name, bytes) in names {
            fs::write(dir.join(name), vec![0u8; *bytes]).unwrap();
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fivemclip-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn picks_the_newest_segments_in_playback_order() {
        let dir = scratch("newest");
        seed(
            &dir,
            &[
                ("seg00000.ts", 10),
                ("seg00001.ts", 10),
                ("seg00002.ts", 10),
                ("seg00003.ts", 10),
            ],
        );

        // Six seconds at two seconds per segment is three, plus one for the
        // partial segment currently being written.
        let mut found = newest_segments(&dir, 6).unwrap();
        assert_eq!(found.len(), 4);

        found.sort_by_key(|(_, m)| *m);
        let names: Vec<String> = found
            .iter()
            .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            ["seg00000.ts", "seg00001.ts", "seg00002.ts", "seg00003.ts"]
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn drops_the_oldest_when_more_buffer_exists_than_asked_for() {
        let dir = scratch("trim");
        seed(
            &dir,
            &[
                ("seg00000.ts", 10),
                ("seg00001.ts", 10),
                ("seg00002.ts", 10),
                ("seg00003.ts", 10),
                ("seg00004.ts", 10),
            ],
        );

        // Two seconds wants one segment plus the partial one.
        let found = newest_segments(&dir, 2).unwrap();
        assert_eq!(found.len(), 2);

        let names: Vec<String> = found
            .iter()
            .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"seg00004.ts".to_string()), "{names:?}");
        assert!(!names.contains(&"seg00000.ts".to_string()), "{names:?}");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ignores_empty_and_unrelated_files() {
        let dir = scratch("filter");
        // ffmpeg opens the next segment before writing to it; a zero-byte file
        // in the concat list makes the whole clip fail.
        seed(&dir, &[("seg00000.ts", 10), ("seg00001.ts", 0)]);
        fs::write(dir.join("concat.txt"), b"not a segment").unwrap();
        fs::write(dir.join("notes.txt"), b"nope").unwrap();

        let found = newest_segments(&dir, 60).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0]
            .0
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("seg00000"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_empty_buffer_is_not_an_error() {
        let dir = scratch("empty");
        assert!(newest_segments(&dir, 30).unwrap().is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn clearing_the_ring_leaves_unrelated_files_alone() {
        let dir = scratch("clear");
        seed(&dir, &[("seg00000.ts", 10)]);
        fs::write(dir.join("keep-me.mp4"), b"a saved clip").unwrap();

        clear_ring(&dir);

        assert!(!dir.join("seg00000.ts").exists());
        assert!(dir.join("keep-me.mp4").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
