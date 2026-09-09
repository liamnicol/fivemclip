use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// How the mic is folded into the saved file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MicMode {
    /// No microphone capture at all.
    Off,
    /// Mic is mixed into the single audio track.
    #[default]
    Mixed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where clips and screenshots land.
    pub output_dir: PathBuf,
    /// Seconds of gameplay kept in the replay buffer.
    pub buffer_seconds: u32,
    pub fps: u32,
    /// Video bitrate in kbit/s.
    pub bitrate_kbps: u32,
    /// DXGI output index. Verified by the user with the preview button rather
    /// than by trusting that our monitor enumeration matches ffmpeg's.
    pub monitor_index: u32,
    pub capture_cursor: bool,
    /// JPEG keeps screenshots small enough to upload instantly; PNG keeps them
    /// pixel-exact. Most people sharing a clip want the former.
    pub screenshot_jpeg: bool,
    /// JPEG quality, 2 (best) to 31 (worst) in ffmpeg's scale.
    pub screenshot_quality: u32,

    pub mic_mode: MicMode,
    /// Gain applied to the mic before mixing, in dB.
    pub mic_gain_db: f32,
    /// Gain applied to system audio before mixing, in dB.
    pub system_gain_db: f32,

    /// Only hold the replay buffer open while FiveM is actually running, so we
    /// are not burning GPU and disk on someone's desktop all day.
    pub only_while_fivem_running: bool,
    pub start_minimized: bool,
    pub autostart: bool,
    /// Cleared until the user has been through first-run setup. The replay
    /// buffer stays off until then: writing hundreds of megabytes to a folder
    /// nobody has chosen yet is not a good first impression.
    pub setup_complete: bool,

    pub hotkey_save_clip: String,
    pub hotkey_screenshot: String,
    pub hotkey_toggle_buffer: String,

    /// Personal ImgBB key. Deliberately per-user: a shared key baked into a
    /// distributed binary gets extracted and rate-limited within a week.
    pub imgbb_api_key: String,
    /// Auto-upload every screenshot and put the link on the clipboard.
    pub imgbb_auto_upload: bool,

    /// Cached winner from the startup pipeline probe. Cleared when hardware or
    /// ffmpeg changes underneath us.
    pub cached_pipeline: Option<String>,
    pub cached_pipeline_fingerprint: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            output_dir: default_output_dir(),
            buffer_seconds: 120,
            fps: 60,
            bitrate_kbps: 30_000,
            monitor_index: 0,
            capture_cursor: false,
            screenshot_jpeg: true,
            screenshot_quality: 3,
            mic_mode: MicMode::Mixed,
            mic_gain_db: 0.0,
            system_gain_db: 0.0,
            only_while_fivem_running: true,
            start_minimized: false,
            autostart: false,
            setup_complete: false,
            hotkey_save_clip: "F9".into(),
            hotkey_screenshot: "F10".into(),
            hotkey_toggle_buffer: "Ctrl+F9".into(),
            imgbb_api_key: String::new(),
            imgbb_auto_upload: false,
            cached_pipeline: None,
            cached_pipeline_fingerprint: None,
        }
    }
}

impl Settings {
    pub fn clips_dir(&self) -> PathBuf {
        self.output_dir.join("Clips")
    }

    pub fn screenshots_dir(&self) -> PathBuf {
        self.output_dir.join("Screenshots")
    }

    /// Scratch space for the ring buffer. Kept beside the output so it lands on
    /// the same (hopefully fast, hopefully roomy) drive the user picked.
    pub fn ring_dir(&self) -> PathBuf {
        self.output_dir.join(".buffer")
    }

    /// Rough worst-case disk footprint of the ring buffer, in bytes. Shown in
    /// the UI so nobody sets a 30 minute buffer at 60 Mbit and fills their SSD.
    pub fn estimated_buffer_bytes(&self) -> u64 {
        // Audio is a rounding error next to the video bitrate.
        let bits = self.bitrate_kbps as u64 * 1000 * self.buffer_seconds as u64;
        (bits / 8) + (bits / 8 / 10)
    }

    pub fn clamp(&mut self) {
        self.buffer_seconds = self.buffer_seconds.clamp(10, 1800);
        self.fps = self.fps.clamp(15, 240);
        self.bitrate_kbps = self.bitrate_kbps.clamp(2_000, 150_000);
        self.mic_gain_db = self.mic_gain_db.clamp(-30.0, 30.0);
        self.screenshot_quality = self.screenshot_quality.clamp(2, 31);
        self.system_gain_db = self.system_gain_db.clamp(-30.0, 30.0);
        if self.output_dir.as_os_str().is_empty() {
            self.output_dir = default_output_dir();
        }
    }
}

pub fn default_output_dir() -> PathBuf {
    let base = dirs_video()
        .or_else(dirs_home)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("FiveMClip")
}

fn dirs_video() -> Option<PathBuf> {
    // USERPROFILE\Videos is right on every consumer Windows install. Reading the
    // shell known-folder path properly would need COM for no practical gain.
    dirs_home().map(|h| h.join("Videos")).filter(|p| p.is_dir())
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_pulls_absurd_values_into_range() {
        let mut s = Settings {
            buffer_seconds: 100_000,
            fps: 1,
            bitrate_kbps: 1,
            screenshot_quality: 99,
            mic_gain_db: 500.0,
            ..Default::default()
        };
        s.clamp();

        assert_eq!(s.buffer_seconds, 1800);
        assert_eq!(s.fps, 15);
        assert_eq!(s.bitrate_kbps, 2_000);
        assert_eq!(s.screenshot_quality, 31);
        assert_eq!(s.mic_gain_db, 30.0);
    }

    #[test]
    fn clamp_restores_a_blank_output_directory() {
        let mut s = Settings {
            output_dir: PathBuf::new(),
            ..Default::default()
        };
        s.clamp();
        assert!(!s.output_dir.as_os_str().is_empty());
    }

    #[test]
    fn buffer_estimate_tracks_bitrate_and_length() {
        let s = Settings {
            bitrate_kbps: 8_000,
            buffer_seconds: 60,
            ..Default::default()
        };
        // 8 Mbit/s for 60 s is 480 Mbit, i.e. 60 MB of video, plus the 10%
        // audio and container margin.
        let video_bytes = 8_000u64 * 1000 * 60 / 8;
        assert_eq!(video_bytes, 60_000_000);
        assert_eq!(s.estimated_buffer_bytes(), video_bytes + video_bytes / 10);
    }

    #[test]
    fn media_directories_sit_under_the_output_directory() {
        let s = Settings {
            output_dir: PathBuf::from("/tmp/example"),
            ..Default::default()
        };
        assert!(s.clips_dir().starts_with(&s.output_dir));
        assert!(s.screenshots_dir().starts_with(&s.output_dir));
        assert_ne!(s.clips_dir(), s.screenshots_dir());
    }

    #[test]
    fn settings_survive_a_round_trip_through_json() {
        let original = Settings {
            mic_mode: MicMode::Off,
            hotkey_save_clip: "Ctrl+Shift+K".into(),
            ..Default::default()
        };

        let json = serde_json::to_string(&original).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.mic_mode, MicMode::Off);
        assert_eq!(restored.hotkey_save_clip, "Ctrl+Shift+K");
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        // Settings files written by an older build must still load.
        let restored: Settings = serde_json::from_str(r#"{"fps": 30}"#).unwrap();
        assert_eq!(restored.fps, 30);
        assert_eq!(restored.buffer_seconds, Settings::default().buffer_seconds);
    }
}
