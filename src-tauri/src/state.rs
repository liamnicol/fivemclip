use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use fivemclip_capture::config::Settings;
use fivemclip_capture::ffmpeg::{self, Pipeline};
use fivemclip_capture::Recorder;
use parking_lot::Mutex;

pub struct AppState {
    pub settings: Mutex<Settings>,
    pub recorder: Mutex<Option<Recorder>>,
    pub ffmpeg: Option<PathBuf>,
    settings_path: PathBuf,
    /// Set while the user has deliberately switched the buffer off, so the
    /// "only while FiveM is running" watchdog does not turn it back on.
    pub manually_stopped: AtomicBool,
}

impl AppState {
    pub fn load(settings_path: PathBuf) -> Self {
        let mut settings = std::fs::read_to_string(&settings_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Settings>(&raw).ok())
            .unwrap_or_default();
        settings.clamp();

        AppState {
            settings: Mutex::new(settings),
            recorder: Mutex::new(None),
            ffmpeg: ffmpeg::find_ffmpeg(),
            settings_path,
            manually_stopped: AtomicBool::new(false),
        }
    }

    pub fn persist(&self) -> Result<(), String> {
        let settings = self.settings.lock().clone();
        if let Some(dir) = self.settings_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
        std::fs::write(&self.settings_path, json).map_err(|e| e.to_string())
    }

    pub fn ffmpeg(&self) -> Result<&PathBuf, String> {
        self.ffmpeg.as_ref().ok_or_else(|| {
            "ffmpeg.exe is missing. Reinstall FiveMClip, or drop ffmpeg.exe into the app's \
             bin folder."
                .to_string()
        })
    }

    /// The encoder pipeline to record with, probing the machine the first time
    /// and whenever the hardware or ffmpeg build changes underneath the cache.
    pub fn resolve_pipeline(&self) -> Result<&'static Pipeline, String> {
        let ffmpeg_path = self.ffmpeg()?;
        let (cached, cached_fp, monitor) = {
            let s = self.settings.lock();
            (
                s.cached_pipeline.clone(),
                s.cached_pipeline_fingerprint.clone(),
                s.monitor_index,
            )
        };
        let fingerprint = ffmpeg::fingerprint(ffmpeg_path, monitor);

        if cached_fp.as_deref() == Some(fingerprint.as_str()) {
            if let Some(p) = cached.as_deref().and_then(ffmpeg::pipeline_by_id) {
                return Ok(p);
            }
        }

        let settings = self.settings.lock().clone();
        let report = ffmpeg::select_pipeline(ffmpeg_path, &settings);
        let chosen = report.chosen.clone().ok_or_else(|| {
            let reasons = report
                .attempts
                .iter()
                .filter_map(|a| a.error.as_ref().map(|e| format!("{}: {e}", a.label)))
                .collect::<Vec<_>>()
                .join("\n");
            format!("No screen capture method worked on this PC.\n\n{reasons}")
        })?;

        {
            let mut s = self.settings.lock();
            s.cached_pipeline = Some(chosen.clone());
            s.cached_pipeline_fingerprint = Some(fingerprint);
        }
        let _ = self.persist();

        ffmpeg::pipeline_by_id(&chosen).ok_or_else(|| "unknown capture pipeline".to_string())
    }

    pub fn ensure_recorder(&self) -> Result<(), String> {
        let pipeline = self.resolve_pipeline()?;
        let ffmpeg_path = self.ffmpeg()?.clone();
        let settings = self.settings.lock().clone();

        let mut guard = self.recorder.lock();
        match guard.as_mut() {
            Some(r) => r.apply_settings(settings, pipeline),
            None => *guard = Some(Recorder::new(ffmpeg_path, settings, pipeline)),
        }
        Ok(())
    }

    pub fn start_buffer(&self) -> Result<(), String> {
        self.ensure_recorder()?;
        self.manually_stopped.store(false, Ordering::Relaxed);
        let mut guard = self.recorder.lock();
        let r = guard.as_mut().ok_or("recorder unavailable")?;
        r.start().map(|_| ())
    }

    pub fn stop_buffer(&self, manual: bool) {
        if manual {
            self.manually_stopped.store(true, Ordering::Relaxed);
        }
        if let Some(r) = self.recorder.lock().as_mut() {
            r.stop();
        }
    }
}
