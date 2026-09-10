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
    /// Put the image on the clipboard as well as on disk. For a region grab
    /// this is usually the whole point - the file is the backup copy.
    pub copy_screenshot_to_clipboard: bool,
    /// Open the redaction editor straight after a region capture.
    ///
    /// Off by default - most screenshots are shared as-is - but for anyone
    /// routinely hiding names, plates or staff chat it saves a trip through
    /// the library every single time.
    pub edit_after_region: bool,
    /// JPEG quality, 2 (best) to 31 (worst) in ffmpeg's scale.
    pub screenshot_quality: u32,

    pub mic_mode: MicMode,
    /// Gain applied to the mic before mixing, in dB.
    pub mic_gain_db: f32,
    /// Gain applied to system audio before mixing, in dB.
    pub system_gain_db: f32,

    /// Stop recording when the drive drops below this many gigabytes free.
    ///
    /// A replay buffer writes continuously and a session recording writes
    /// without bound, so without a floor the app will happily fill someone's
    /// system drive and take Windows down with it.
    pub min_free_gb: u32,
    /// Delete the oldest clips and screenshots once they exceed
    /// `max_library_gb`. Never touches session recordings.
    /// Off by default: silently removing someone's recordings is not something
    /// to opt people into.
    pub auto_prune: bool,
    pub max_library_gb: u32,

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
    /// Drag-a-rectangle capture, the way Greenshot and ShareX do it.
    pub hotkey_region: String,
    /// Start and stop a whole-session recording.
    pub hotkey_session: String,
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
            copy_screenshot_to_clipboard: true,
            edit_after_region: false,
            screenshot_quality: 3,
            mic_mode: MicMode::Mixed,
            mic_gain_db: 0.0,
            system_gain_db: 0.0,
            min_free_gb: 10,
            auto_prune: false,
            max_library_gb: 50,
            only_while_fivem_running: true,
            start_minimized: false,
            autostart: false,
            setup_complete: false,
            hotkey_save_clip: "F9".into(),
            hotkey_screenshot: "F10".into(),
            hotkey_region: "F11".into(),
            hotkey_session: "F8".into(),
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

    /// Whole-session recordings, kept apart from clips: they are hours long
    /// and nobody wants them mixed in with the ten-second highlights.
    pub fn sessions_dir(&self) -> PathBuf {
        self.output_dir.join("Sessions")
    }

    /// Scratch space for the ring buffer. Kept beside the output so it lands on
    /// the same (hopefully fast, hopefully roomy) drive the user picked.
    pub fn ring_dir(&self) -> PathBuf {
        self.output_dir.join(".buffer")
    }

    /// Rough worst-case disk footprint of the ring buffer, in bytes. Shown in
    /// the UI so nobody sets a 30 minute buffer at 60 Mbit and fills their SSD.
    pub fn min_free_bytes(&self) -> u64 {
        self.min_free_gb as u64 * 1_000_000_000
    }

    pub fn max_library_bytes(&self) -> u64 {
        self.max_library_gb as u64 * 1_000_000_000
    }

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
        // A floor below a couple of gigabytes is not a floor: Windows itself
        // starts misbehaving long before a disk is genuinely full.
        self.min_free_gb = self.min_free_gb.clamp(2, 500);
        self.max_library_gb = self.max_library_gb.clamp(1, 10_000);
        self.system_gain_db = self.system_gain_db.clamp(-30.0, 30.0);
        if self.output_dir.as_os_str().is_empty() {
            self.output_dir = default_output_dir();
        }
    }
}

/// The folder a portable copy lives in, if this is one.
///
/// Portability is declared by a marker file shipped in the zip rather than
/// inferred from where the executable happens to sit. Guessing - "am I under
/// Program Files?" - gets it wrong for anyone who installs somewhere unusual,
/// and getting it wrong means writing settings to the wrong place.
pub fn portable_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    dir.join(PORTABLE_MARKER)
        .is_file()
        .then(|| dir.to_path_buf())
}

pub const PORTABLE_MARKER: &str = "portable.txt";

pub fn is_portable() -> bool {
    portable_root().is_some()
}

/// Where settings live: beside the executable when portable, otherwise the
/// usual per-user config directory.
///
/// A portable copy that wrote to %APPDATA% would silently share configuration
/// with an installed one and leave settings behind when its folder is deleted,
/// which is the one thing portable software must not do.
pub fn settings_path(config_dir: &std::path::Path) -> PathBuf {
    match portable_root() {
        Some(root) => root.join("settings.json"),
        None => config_dir.join("settings.json"),
    }
}

pub fn default_output_dir() -> PathBuf {
    // Recordings belong inside the portable folder too, so deleting it really
    // does leave nothing behind. The setup screen asks on first run, so anyone
    // who would rather keep clips on a roomier drive can say so.
    if let Some(root) = portable_root() {
        return root.join("Recordings");
    }

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

#[cfg(test)]
mod disk_tests {
    use super::*;

    #[test]
    fn free_space_floor_stays_sane() {
        let mut s = Settings {
            min_free_gb: 0,
            ..Default::default()
        };
        s.clamp();
        // Zero would let the buffer run a drive to genuinely full, which takes
        // Windows down with it.
        assert!(s.min_free_gb >= 2);

        let mut s = Settings {
            min_free_gb: 99_999,
            ..Default::default()
        };
        s.clamp();
        assert!(s.min_free_gb <= 500);
    }

    #[test]
    fn thresholds_convert_to_bytes() {
        let s = Settings {
            min_free_gb: 10,
            max_library_gb: 50,
            ..Default::default()
        };
        assert_eq!(s.min_free_bytes(), 10_000_000_000);
        assert_eq!(s.max_library_bytes(), 50_000_000_000);
    }

    #[test]
    fn pruning_is_off_unless_asked_for() {
        assert!(!Settings::default().auto_prune);
    }
}

#[cfg(test)]
mod portable_tests {
    use super::*;

    #[test]
    fn an_ordinary_build_is_not_portable() {
        // The test binary has no marker beside it, which is the same situation
        // an installed copy is in.
        assert!(!is_portable());
        assert!(portable_root().is_none());
    }

    #[test]
    fn settings_fall_back_to_the_config_directory() {
        let config = PathBuf::from("/tmp/config");
        assert_eq!(settings_path(&config), config.join("settings.json"));
    }
}
