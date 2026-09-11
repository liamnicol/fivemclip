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
use std::time::{Duration, Instant};

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
    session: Option<Session>,
}

/// A whole-session recording in progress.
///
/// This is not a second encode. The replay buffer is already writing every
/// frame to disk as short segments; a session recording just means "stop
/// recycling them from here, and remember where here was". At the end the
/// segments from that point are concatenated. One encode, no extra GPU cost,
/// and no second ffmpeg process competing for the encoder.
#[derive(Debug, Clone)]
struct Session {
    /// Segments written at or after this instant belong to the session.
    /// Compared against file mtimes, so it has to be wall clock rather than
    /// an Instant.
    started_at: std::time::SystemTime,
    label: String,
    /// Offsets, in seconds from `started_at`, that the user marked as worth
    /// coming back to.
    ///
    /// Measured the same way `session_seconds` is, so a marker lines up with
    /// the duration the UI was showing when it was dropped. The stitched file
    /// can start up to one segment later than `started_at`, so these are
    /// "roughly here" rather than frame-exact - which is all a three hour
    /// recording needs, and the trimmer is there for the rest.
    markers: Vec<f64>,
    /// Size on disk, refreshed occasionally rather than on every status poll.
    ///
    /// Status is polled about once a second and the ring grows without bound
    /// during a session, so measuring it each time means thousands of stat
    /// calls a second after a couple of hours - while holding the recorder lock.
    cached_bytes: u64,
    measured_at: Option<Instant>,
}

struct Running {
    child: Child,
    stop: Arc<AtomicBool>,
    audio_thread: Option<JoinHandle<()>>,
    started: Instant,
    /// Non-fatal problems worth surfacing, e.g. "recording without audio".
    warnings: Vec<String>,
}

/// A finished session: where it was written, and the moments marked during it.
#[derive(Debug, Clone)]
pub struct SavedSession {
    pub path: PathBuf,
    pub markers: Vec<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RecorderStatus {
    pub running: bool,
    pub seconds_buffered: u32,
    pub pipeline: String,
    pub has_audio: bool,
    pub warnings: Vec<String>,
    pub session_active: bool,
    pub session_seconds: u64,
    pub session_bytes: u64,
    pub session_markers: usize,
}

impl Recorder {
    pub fn new(ffmpeg: PathBuf, settings: Settings, pipeline: &'static Pipeline) -> Self {
        Self {
            ffmpeg,
            settings,
            pipeline,
            running: None,
            session: None,
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
        let session_seconds = self
            .session
            .as_ref()
            .and_then(|s| s.started_at.elapsed().ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let session_bytes = self.session_bytes_on_disk();

        RecorderStatus {
            running,
            seconds_buffered: seconds,
            pipeline: self.pipeline.label.to_string(),
            has_audio,
            warnings,
            session_active: self.session.is_some(),
            session_seconds,
            session_bytes,
            session_markers: self.session_marker_count(),
        }
    }

    pub fn session_active(&self) -> bool {
        self.session.is_some()
    }

    /// Size of the running session, remeasured at most every few seconds.
    fn session_bytes_on_disk(&mut self) -> u64 {
        const REMEASURE_AFTER: Duration = Duration::from_secs(5);

        let ring = self.settings.ring_dir();
        let Some(session) = self.session.as_mut() else {
            return 0;
        };
        if session
            .measured_at
            .map(|at| at.elapsed() < REMEASURE_AFTER)
            .unwrap_or(false)
        {
            return session.cached_bytes;
        }

        session.cached_bytes = segments_since(&ring, session.started_at)
            .into_iter()
            .filter_map(|(path, _)| fs::metadata(path).ok())
            .map(|m| m.len())
            .sum();
        session.measured_at = Some(Instant::now());
        session.cached_bytes
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
        // A restart in the middle of a session must keep what the session has
        // recorded so far.
        if self.session.is_none() {
            clear_ring(&ring);
        }

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

        // Windows keeps orphaned children alive when their parent dies, so
        // without this a crash leaves ffmpeg recording forever.
        crate::reaper::adopt(&child);

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

        a.extend([
            "-f".into(),
            "segment".into(),
            "-segment_time".into(),
            SEGMENT_SECONDS.to_string(),
        ]);

        if self.session.is_some() {
            // No wrap: during a session every segment is kept, because it is
            // part of the recording rather than just recent history. Growth is
            // bounded by the disk guard, not by recycling.
            //
            // Numbering continues past whatever is already on disk so the
            // replay buffer keeps its existing segments across the restart -
            // starting a session should not throw away the last few minutes.
            a.extend([
                "-segment_start_number".into(),
                next_segment_number(ring).to_string(),
            ]);
        } else {
            // Enough slots for the whole buffer plus a couple in hand, so the
            // segment currently being written never clobbers one we still need.
            let wrap = s.buffer_seconds.div_ceil(SEGMENT_SECONDS) + 2;
            a.extend(["-segment_wrap".into(), wrap.to_string()]);
        }

        a.extend([
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

    /// Begin keeping every segment from now on.
    ///
    /// Restarting ffmpeg costs about a second of footage. That is the price of
    /// changing the segment muxer's recycling behaviour mid-stream, and it is
    /// far cheaper than running a second encoder for the whole session.
    pub fn start_session(&mut self) -> Result<(), String> {
        if self.session.is_some() {
            return Ok(());
        }
        if !self.is_running() {
            return Err("Start the replay buffer before recording a session.".into());
        }

        self.session = Some(Session {
            started_at: std::time::SystemTime::now(),
            label: chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string(),
            markers: Vec::new(),
            cached_bytes: 0,
            measured_at: None,
        });

        self.stop();
        if let Err(e) = self.start() {
            self.session = None;
            return Err(e);
        }
        Ok(())
    }

    /// Mark the current moment as worth coming back to.
    ///
    /// Returns where in the session it landed, so the UI can say "marked at
    /// 1h42" rather than just "marked".
    pub fn mark_session(&mut self) -> Result<f64, String> {
        let session = self
            .session
            .as_mut()
            .ok_or("Markers are for session recordings - start one first.")?;
        let at = session
            .started_at
            .elapsed()
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        // A double press is a slip, not two moments a second apart.
        if session.markers.last().is_some_and(|last| at - last < 1.0) {
            return Ok(at);
        }
        session.markers.push(at);
        Ok(at)
    }

    pub fn session_marker_count(&self) -> usize {
        self.session.as_ref().map(|s| s.markers.len()).unwrap_or(0)
    }

    /// Finish the session and write it out.
    ///
    /// MP4, the same as a clip. This used to be Matroska, on the reasoning that
    /// MP4 writes its index last and so a session ending in a crash would leave
    /// an unplayable file - but a session is never written to this file while
    /// it records. It lives as MPEG-TS segments in the ring and is only
    /// assembled here, after recording has already stopped, so the container
    /// choice buys no crash safety at all. What it did cost was a file Explorer
    /// will not thumbnail and half the web will not accept.
    ///
    /// Sessions already on disk keep working: the library still lists .mkv.
    pub fn stop_session(&mut self) -> Result<SavedSession, String> {
        let session = self.session.take().ok_or("No session is being recorded.")?;

        // Killing ffmpeg truncates whatever segment is mid-write, so up to
        // SEGMENT_SECONDS of the tail is lost. Shutting it down gracefully
        // would need a control channel we do not have, since stdin already
        // carries the audio.
        let was_running = self.is_running();
        self.stop();

        let ring = self.settings.ring_dir();
        let segments = segments_since(&ring, session.started_at);
        if segments.is_empty() {
            if was_running {
                let _ = self.start();
            }
            return Err("That session was too short to save.".into());
        }

        let dir = self.settings.sessions_dir();
        fs::create_dir_all(&dir).map_err(|e| format!("could not create sessions folder: {e}"))?;
        let out = dir.join(format!("Session_{}.mp4", session.label));

        // No faststart. It moves the index to the front by rewriting the whole
        // file, which is nothing on a ten second clip and minutes on a session
        // that ran all evening - and a session is watched off the local disk,
        // not streamed while it downloads.
        match concat_segments(&self.ffmpeg, &ring, &segments, &out, false) {
            Ok(()) => {
                if was_running {
                    let _ = self.start();
                }
                Ok(SavedSession {
                    path: out,
                    markers: session.markers,
                })
            }
            Err(e) => {
                // Put the session back before restarting. Without this the
                // restart runs with no session active, which clears the ring -
                // and the ring is where the whole recording lives. A failed
                // save would delete hours of footage, and the likeliest reason
                // to fail is not enough room for the output, which is exactly
                // when the source needs to survive.
                let _ = fs::remove_file(&out);
                self.session = Some(session);
                if was_running {
                    let _ = self.start();
                }
                Err(format!(
                    "Could not save the session: {e}. The recording is still on disk - \
                     free up some space and try again."
                ))
            }
        }
    }

    /// Abandon the session without writing it out.
    pub fn discard_session(&mut self) {
        if self.session.take().is_some() {
            let was_running = self.is_running();
            self.stop();
            if was_running {
                let _ = self.start();
            }
        }
    }

    /// Concatenate the newest `clip_seconds` of buffer into an MP4.
    ///
    /// The length is read from the recorder's own settings rather than passed
    /// in. It used to be an argument, and three separate callers each had to
    /// remember which field to read - the tray item was still passing
    /// `buffer_seconds` long after the hotkey and the button were fixed, so
    /// every clip saved from the tray was the entire buffer.
    pub fn save_clip(&mut self) -> Result<PathBuf, String> {
        if !self.is_running() {
            return Err("The replay buffer is not running.".into());
        }
        let s = &self.settings;
        let seconds = s.clip_seconds.clamp(SEGMENT_SECONDS, s.buffer_seconds);
        let ring = s.ring_dir();

        let mut segments = newest_segments(&ring, seconds)?;
        if segments.is_empty() {
            return Err("Nothing buffered yet - give it a few seconds.".into());
        }
        segments.sort_by_key(|(_, m)| *m);

        let clips = s.clips_dir();
        fs::create_dir_all(&clips).map_err(|e| format!("could not create clips folder: {e}"))?;

        let out = clips.join(format!(
            "Clip_{}.mp4",
            chrono::Local::now().format("%Y-%m-%d_%H-%M-%S")
        ));

        // Faststart here: a clip is short enough for the extra pass to be
        // free, and it is the file that gets uploaded and streamed.
        concat_segments(&self.ffmpeg, &ring, &segments, &out, true)
            .map_err(|e| format!("Could not save the clip: {e}"))?;

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
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            // Concat lists are named after their output now, so matching the
            // old literal "concat.txt" left one behind per interrupted save.
            if p.extension().map(|x| x == "ts").unwrap_or(false)
                || (name.starts_with("concat") && name.ends_with(".txt"))
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

/// The number ffmpeg should give the next segment it writes.
///
/// Segments already on disk are the replay buffer's recent history. Numbering
/// past them means a restart - which is how a session begins - does not
/// overwrite the few minutes the user already had buffered.
fn next_segment_number(ring: &Path) -> u32 {
    let Ok(entries) = fs::read_dir(ring) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if !path.extension().map(|x| x == "ts").unwrap_or(false) {
                return None;
            }
            path.file_stem()?
                .to_str()?
                .strip_prefix("seg")?
                .parse::<u32>()
                .ok()
        })
        .max()
        .map(|highest| highest + 1)
        .unwrap_or(0)
}

/// Segments written at or after `since`, oldest first.
fn segments_since(
    ring: &Path,
    since: std::time::SystemTime,
) -> Vec<(PathBuf, std::time::SystemTime)> {
    let Ok(entries) = fs::read_dir(ring) else {
        return Vec::new();
    };
    let mut files: Vec<(PathBuf, std::time::SystemTime)> = entries
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .map(|x| x.eq_ignore_ascii_case("ts"))
                .unwrap_or(false)
        })
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if meta.len() == 0 {
                return None;
            }
            let modified = meta.modified().ok()?;
            // A segment finished before the session began belongs to the
            // replay buffer's history, not to the recording.
            (modified >= since).then_some((e.path(), modified))
        })
        .collect();
    files.sort_by_key(|(_, modified)| *modified);
    files
}

/// Stitch segments into one file without re-encoding.
///
/// The output container is chosen by extension: MP4 for clips, which people
/// upload and scrub, and Matroska for sessions, which need to survive being
/// interrupted.
fn concat_segments(
    ffmpeg_path: &Path,
    ring: &Path,
    segments: &[(PathBuf, std::time::SystemTime)],
    out: &Path,
    faststart: bool,
) -> Result<(), String> {
    let list_path = ring.join(format!(
        "concat-{}.txt",
        out.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "list".into())
    ));

    let mut list = String::new();
    for (path, _) in segments {
        // The concat demuxer treats backslashes as escapes, and a single quote
        // has to be broken out of the quoting entirely.
        let escaped = path
            .to_string_lossy()
            .replace('\\', "/")
            .replace('\'', "'\\''");
        list.push_str(&format!("file '{escaped}'\n"));
    }
    fs::write(&list_path, list).map_err(|e| format!("could not stage the file list: {e}"))?;

    let wants_mp4 = out
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("mp4"))
        .unwrap_or(false);

    let mut command = ffmpeg::command(ffmpeg_path);
    command.args([
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
    ]);
    command.arg(&list_path).args(["-c", "copy"]);
    if wants_mp4 {
        // AAC carried in MPEG-TS uses ADTS framing, which MP4 rejects.
        command.args(["-bsf:a", "aac_adtstoasc"]);
        if faststart {
            command.args(["-movflags", "+faststart"]);
        }
    }

    let status = command
        .arg(out)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("could not run ffmpeg: {e}"))?;

    let _ = fs::remove_file(&list_path);

    if status.status.success() {
        Ok(())
    } else {
        Err(ffmpeg::explain(&String::from_utf8_lossy(&status.stderr)))
    }
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
    fn segment_numbering_continues_past_what_is_already_there() {
        let dir = scratch("numbering");
        assert_eq!(next_segment_number(&dir), 0, "empty ring starts at zero");

        seed(&dir, &[("seg00000.ts", 10), ("seg00007.ts", 10)]);
        // Starting a session restarts ffmpeg. Numbering from zero again would
        // overwrite the buffered history the user still expects to be able to
        // clip from.
        assert_eq!(next_segment_number(&dir), 8);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_session_only_claims_segments_written_after_it_began() {
        let dir = scratch("session-window");
        seed(&dir, &[("seg00000.ts", 10), ("seg00001.ts", 10)]);

        let boundary = std::time::SystemTime::now();
        std::thread::sleep(Duration::from_millis(40));
        seed(&dir, &[("seg00002.ts", 10), ("seg00003.ts", 10)]);

        let claimed = segments_since(&dir, boundary);
        let names: Vec<String> = claimed
            .iter()
            .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["seg00002.ts", "seg00003.ts"], "{names:?}");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn clearing_the_ring_removes_concat_lists_whatever_they_are_called() {
        let dir = scratch("stale-lists");
        seed(&dir, &[("seg00000.ts", 10)]);
        // Named after their output since clips and sessions started sharing
        // the concat path; the old code matched only "concat.txt" and leaked
        // one file per interrupted save.
        fs::write(dir.join("concat-Clip_2026-01-01_00-00-00.txt"), b"x").unwrap();
        fs::write(dir.join("concat.txt"), b"x").unwrap();
        fs::write(dir.join("keep-me.mp4"), b"a saved clip").unwrap();

        clear_ring(&dir);

        assert!(!dir.join("concat-Clip_2026-01-01_00-00-00.txt").exists());
        assert!(!dir.join("concat.txt").exists());
        assert!(dir.join("keep-me.mp4").exists());

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
