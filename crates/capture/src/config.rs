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

/// An S3-compatible bucket the user owns.
///
/// Their storage, their bill, their credentials. The alternative - us hosting
/// clips - means accounts, subscriptions, quotas, deletion policies and being
/// answerable for other people's video, none of which this app is.
///
/// Every field is empty until configured, and `is_configured` is what gates the
/// feature rather than an extra tick box that can disagree with it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct S3Target {
    /// e.g. https://<account>.r2.cloudflarestorage.com
    pub endpoint: String,
    pub bucket: String,
    /// "auto" for R2. S3 itself wants a real region, and the signature carries
    /// it, so a wrong one is rejected rather than ignored.
    pub region: String,
    pub access_key_id: String,
    /// A secret in the same way the webhooks are, except this one can write to
    /// and delete from their bucket.
    pub secret_access_key: String,
    /// The domain in front of the bucket, if there is one. A bucket endpoint is
    /// not normally readable by whoever is sent the link.
    pub public_base: String,
    /// Key prefix, so clips do not land in the root of a shared bucket.
    pub prefix: String,
}

impl S3Target {
    /// Everything a signed PUT needs. `public_base` and `prefix` are optional,
    /// so they are not part of this.
    pub fn is_configured(&self) -> bool {
        [
            &self.endpoint,
            &self.bucket,
            &self.access_key_id,
            &self.secret_access_key,
        ]
        .iter()
        .all(|f| !f.trim().is_empty())
    }
}

/// A rectangle over the frame, as fractions of its width and height.
///
/// Fractions rather than pixels so one saved region stays correct across a
/// resolution change, a windowed capture and a clip that has been scaled on the
/// way out - a pixel rectangle picked on a 1440p monitor covers the wrong part
/// of a 1080p export, and covering the wrong part is indistinguishable from not
/// covering anything.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChatRegion {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl ChatRegion {
    /// Build from pixels measured on a frame of a known size.
    pub fn from_pixels(x: u32, y: u32, w: u32, h: u32, frame_w: u32, frame_h: u32) -> Option<Self> {
        if frame_w == 0 || frame_h == 0 || w == 0 || h == 0 {
            return None;
        }
        Some(Self {
            x: x as f32 / frame_w as f32,
            y: y as f32 / frame_h as f32,
            w: w as f32 / frame_w as f32,
            h: h as f32 / frame_h as f32,
        })
    }

    /// Keep the rectangle on the frame, and say whether anything is left.
    ///
    /// A region dragged past the edge of the screen, or one saved by an older
    /// build with a different idea of the units, must not silently become a box
    /// over the middle of the picture.
    fn tidy(&mut self) -> bool {
        if ![self.x, self.y, self.w, self.h]
            .iter()
            .all(|v| v.is_finite())
        {
            return false;
        }
        self.x = self.x.clamp(0.0, 1.0);
        self.y = self.y.clamp(0.0, 1.0);
        self.w = self.w.clamp(0.0, 1.0 - self.x);
        self.h = self.h.clamp(0.0, 1.0 - self.y);
        // Below about a twentieth of the frame each way there is no chat box
        // there, just a misdrag.
        self.w > 0.005 && self.h > 0.005
    }
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// One Discord channel to post to.
///
/// The limit is per target rather than global: a webhook points at a channel in
/// a particular server, and how large an upload that server accepts depends on
/// its boost level. Somebody posting clips to their own server and screenshots
/// to a friend's has two different ceilings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscordTarget {
    /// What to call it in the menu. "Clips", "Staff", "#highlights".
    pub name: String,
    pub url: String,
    pub limit_mb: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where clips and screenshots land.
    pub output_dir: PathBuf,
    /// Seconds of gameplay kept in the replay buffer.
    pub buffer_seconds: u32,
    /// How much of that buffer a saved clip actually contains.
    ///
    /// Separate from `buffer_seconds` on purpose. Saving the whole buffer means
    /// a long buffer - which people set so they never miss anything - produces a
    /// multi-gigabyte file on every single press, so being careful about missing
    /// a moment used to cost you the disk.
    pub clip_seconds: u32,
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

    /// Only hold the replay buffer open while one of `trigger_processes` is
    /// running, so we are not burning GPU and disk on someone's desktop all day.
    pub only_while_fivem_running: bool,
    /// Start a session recording the moment a trigger process appears, and save
    /// it when that process goes away.
    ///
    /// Off by default, deliberately. A session writes without bound - at the
    /// default 30 Mbps that is about 13 GB an hour - so an evening's play is
    /// tens of gigabytes. That is a fine trade for someone who wants it and a
    /// nasty surprise for someone who did not ask.
    pub auto_session: bool,

    /// Executables whose presence means "record now".
    ///
    /// Nothing in the capture path is FiveM-specific - Desktop Duplication
    /// takes the whole screen - so this is the only thing tying the app to one
    /// game, and there is no reason for it to be a hardcoded list.
    pub trigger_processes: Vec<String>,
    pub start_minimized: bool,
    /// On by default, and offered as a tick on the first-run screen.
    ///
    /// A replay buffer is only any use if it was already running when the thing
    /// worth keeping happened. Launching the app first, every time, is exactly
    /// the step people forget - and then discover they have forgotten it right
    /// after the one moment they wanted.
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
    /// Drop a marker into the running session.
    pub hotkey_marker: String,

    /// Personal ImgBB key. Deliberately per-user: a shared key baked into a
    /// distributed binary gets extracted and rate-limited within a week.
    pub imgbb_api_key: String,
    /// Auto-upload every screenshot and put the link on the clipboard.
    pub imgbb_auto_upload: bool,

    /// Channels to post to. Each webhook is one channel, so sending clips to
    /// one place and screenshots to another means more than one of these.
    ///
    /// Every URL here is a secret in the same way the ImgBB key is: anyone
    /// holding one can post to that channel.
    pub discord_targets: Vec<DiscordTarget>,

    /// The single webhook this used to be, kept only so an existing settings
    /// file migrates into `discord_targets` on load. Never written back.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub discord_webhook: String,
    /// The single limit this used to be, kept for the same migration.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub discord_limit_mb: u32,

    /// Where uploads go, when the user has their own bucket.
    pub s3: S3Target,

    /// Black out the chat box when a clip is exported.
    ///
    /// Off by default. It costs a re-encode and it is irreversible, so it is
    /// not something to opt people into - but for anyone whose server forbids
    /// showing staff chat, reports or OOC, it is the difference between
    /// sharing a clip and not.
    pub hide_chat: bool,
    /// Where the chat box sits, picked by dragging over a frozen frame. Without
    /// one, `hide_chat` has nothing to cover and does nothing.
    pub chat_region: Option<ChatRegion>,

    /// Last version whose "what's new" notes the user has seen. Empty on a
    /// fresh install, which is why the splash is gated on `setup_complete`
    /// too - nobody wants a changelog for software they installed a minute ago.
    pub last_seen_version: String,

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
            clip_seconds: 60,
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
            auto_session: false,
            only_while_fivem_running: true,
            trigger_processes: vec!["FiveM".into(), "RedM".into()],
            start_minimized: false,
            autostart: true,
            setup_complete: false,
            hotkey_save_clip: "F9".into(),
            hotkey_screenshot: "F10".into(),
            hotkey_region: "F11".into(),
            hotkey_session: "F8".into(),
            hotkey_toggle_buffer: "Ctrl+F9".into(),
            hotkey_marker: "F7".into(),
            imgbb_api_key: String::new(),
            imgbb_auto_upload: false,
            s3: S3Target::default(),
            hide_chat: false,
            chat_region: None,
            discord_targets: Vec::new(),
            discord_webhook: String::new(),
            discord_limit_mb: 0,
            last_seen_version: String::new(),
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

    /// Fold the old single-webhook settings into the list.
    ///
    /// Runs on every load and is idempotent: once the list has the target, the
    /// old fields are cleared and skipped when serialising, so this stops
    /// happening. Without it, everyone who set a webhook before this existed
    /// would open the app to an empty list and assume it had been lost.
    fn migrate_discord(&mut self) {
        let old = std::mem::take(&mut self.discord_webhook);
        let limit = std::mem::take(&mut self.discord_limit_mb);
        if old.trim().is_empty() {
            return;
        }
        if self.discord_targets.iter().any(|t| t.url == old) {
            return;
        }
        self.discord_targets.push(DiscordTarget {
            name: "Discord".into(),
            url: old,
            limit_mb: if limit == 0 { 10 } else { limit },
        });
    }

    pub fn clamp(&mut self) {
        self.buffer_seconds = self.buffer_seconds.clamp(10, 1800);
        // Never longer than there is buffer to take it from, and never shorter
        // than one segment - below that there is nothing to concatenate.
        self.clip_seconds = self.clip_seconds.clamp(5, self.buffer_seconds);
        self.fps = self.fps.clamp(15, 240);
        self.bitrate_kbps = self.bitrate_kbps.clamp(2_000, 150_000);
        self.mic_gain_db = self.mic_gain_db.clamp(-30.0, 30.0);
        self.screenshot_quality = self.screenshot_quality.clamp(2, 31);
        self.trigger_processes.retain(|p| !p.trim().is_empty());
        // An empty list with the toggle on would mean "record when nothing is
        // running", which is never what anyone meant.
        if self.trigger_processes.is_empty() {
            self.only_while_fivem_running = false;
        }
        // A floor below a couple of gigabytes is not a floor: Windows itself
        // starts misbehaving long before a disk is genuinely full.
        self.min_free_gb = self.min_free_gb.clamp(2, 500);
        // A region that cannot be made sense of is dropped rather than
        // corrected: a box over the wrong part of the clip looks like the
        // feature working while hiding nothing.
        if let Some(mut region) = self.chat_region {
            self.chat_region = region.tidy().then_some(region);
        }

        // Trimmed on the way in: a pasted endpoint or key with a trailing
        // newline signs correctly and then fails against a host that does not
        // exist, which is a miserable thing to debug from an error message.
        for field in [
            &mut self.s3.endpoint,
            &mut self.s3.bucket,
            &mut self.s3.region,
            &mut self.s3.access_key_id,
            &mut self.s3.secret_access_key,
            &mut self.s3.public_base,
            &mut self.s3.prefix,
        ] {
            *field = field.trim().to_string();
        }
        if self.s3.region.is_empty() {
            // What R2 wants, and the most likely bucket behind this.
            self.s3.region = "auto".into();
        }

        self.migrate_discord();
        for target in &mut self.discord_targets {
            // The most conservative of Discord's tiers is 10 MB: too small only
            // costs a needless re-encode, too large means an upload that is
            // refused after the wait.
            target.limit_mb = target.limit_mb.clamp(1, 500);
            if target.name.trim().is_empty() {
                target.name = "Discord".into();
            }
        }
        self.discord_targets.retain(|t| !t.url.trim().is_empty());
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

#[cfg(test)]
mod clip_length_tests {
    use super::*;

    /// A clip used to be the whole buffer, so a 20 minute buffer meant a 20
    /// minute file on every press. The two lengths are independent now.
    #[test]
    fn a_long_buffer_does_not_force_a_long_clip() {
        let mut s = Settings {
            buffer_seconds: 1200,
            clip_seconds: 60,
            ..Default::default()
        };
        s.clamp();
        assert_eq!(s.buffer_seconds, 1200);
        assert_eq!(s.clip_seconds, 60);
    }

    #[test]
    fn a_clip_cannot_outrun_its_buffer() {
        let mut s = Settings {
            buffer_seconds: 30,
            clip_seconds: 600,
            ..Default::default()
        };
        s.clamp();
        assert_eq!(s.clip_seconds, 30);
    }

    #[test]
    fn the_default_clip_is_a_minute_not_the_whole_buffer() {
        let s = Settings::default();
        assert_eq!(s.clip_seconds, 60);
        assert!(s.clip_seconds < s.buffer_seconds);
    }
}

#[cfg(test)]
mod autostart_default_tests {
    use super::*;

    /// A fresh install starts with Windows unless the user unticks it on the
    /// first-run screen. The buffer is worthless if it was not already running.
    #[test]
    fn autostart_is_on_for_a_fresh_install() {
        assert!(Settings::default().autostart);
    }

    /// Someone who turned it off must stay off. serde(default) only fills in
    /// fields that are absent, so a stored `false` has to survive the default
    /// flipping to true - otherwise an update silently re-enables it.
    #[test]
    fn turning_it_off_survives_the_default_changing() {
        let stored = r#"{"autostart": false}"#;
        let restored: Settings = serde_json::from_str(stored).expect("parses");
        assert!(!restored.autostart);
    }
}

#[cfg(test)]
mod chat_region_tests {
    use super::*;

    /// Stored as fractions so the region survives a resolution change. A
    /// rectangle picked on 1440p and applied as pixels to 1080p covers the
    /// wrong part of the clip, which looks like the feature working.
    #[test]
    fn pixels_become_fractions_of_the_frame() {
        let r = ChatRegion::from_pixels(0, 750, 500, 270, 1920, 1080).expect("valid");
        assert!((r.x - 0.0).abs() < 1e-6);
        assert!((r.y - 0.6944).abs() < 1e-3);
        assert!((r.w - 0.2604).abs() < 1e-3);
        assert!((r.h - 0.25).abs() < 1e-3);
    }

    /// The same drag on two different monitors describes the same fraction of
    /// the picture, which is the whole reason for the units.
    #[test]
    fn the_same_relative_drag_matches_across_resolutions() {
        let hd = ChatRegion::from_pixels(0, 750, 500, 270, 1920, 1080).expect("valid");
        let qhd = ChatRegion::from_pixels(0, 1000, 667, 360, 2560, 1440).expect("valid");
        assert!((hd.x - qhd.x).abs() < 0.01);
        assert!((hd.y - qhd.y).abs() < 0.01);
        assert!((hd.w - qhd.w).abs() < 0.01);
        assert!((hd.h - qhd.h).abs() < 0.01);
    }

    #[test]
    fn a_zero_sized_drag_is_not_a_region() {
        assert!(ChatRegion::from_pixels(10, 10, 0, 50, 1920, 1080).is_none());
        assert!(ChatRegion::from_pixels(10, 10, 50, 0, 1920, 1080).is_none());
    }

    /// A frame with no size means the overlay's image never loaded. Dividing by
    /// it would produce infinities and a box over the whole clip.
    #[test]
    fn a_frame_with_no_size_is_refused() {
        assert!(ChatRegion::from_pixels(10, 10, 50, 50, 0, 1080).is_none());
        assert!(ChatRegion::from_pixels(10, 10, 50, 50, 1920, 0).is_none());
    }

    #[test]
    fn a_region_hanging_off_the_edge_is_pulled_back_on() {
        let mut s = Settings {
            chat_region: Some(ChatRegion {
                x: 0.8,
                y: 0.9,
                w: 0.5,
                h: 0.4,
            }),
            ..Default::default()
        };
        s.clamp();
        let r = s.chat_region.expect("kept");
        assert!((r.x + r.w) <= 1.0001, "{r:?}");
        assert!((r.y + r.h) <= 1.0001, "{r:?}");
    }

    /// Dropped rather than corrected. A rectangle that cannot be made sense of
    /// has no relationship to where the chat is, and a black box over the
    /// middle of the picture is worse than no box at all.
    #[test]
    fn nonsense_is_dropped_rather_than_guessed_at() {
        for bad in [
            ChatRegion {
                x: f32::NAN,
                y: 0.5,
                w: 0.3,
                h: 0.2,
            },
            ChatRegion {
                x: 0.1,
                y: 0.5,
                w: f32::INFINITY,
                h: 0.2,
            },
            // Entirely off the frame: clamping leaves nothing.
            ChatRegion {
                x: 1.0,
                y: 0.5,
                w: 0.3,
                h: 0.2,
            },
            // A misdrag, not a chat box.
            ChatRegion {
                x: 0.2,
                y: 0.2,
                w: 0.001,
                h: 0.001,
            },
        ] {
            let mut s = Settings {
                chat_region: Some(bad),
                ..Default::default()
            };
            s.clamp();
            assert!(s.chat_region.is_none(), "should have dropped {bad:?}");
        }
    }

    #[test]
    fn a_good_region_survives_a_round_trip() {
        let region = ChatRegion {
            x: 0.01,
            y: 0.68,
            w: 0.39,
            h: 0.25,
        };
        let mut s = Settings {
            chat_region: Some(region),
            hide_chat: true,
            ..Default::default()
        };
        s.clamp();
        let json = serde_json::to_string(&s).expect("serialises");
        let mut back: Settings = serde_json::from_str(&json).expect("parses");
        back.clamp();
        assert_eq!(back.chat_region, Some(region));
        assert!(back.hide_chat);
    }
}

#[cfg(test)]
mod discord_target_tests {
    use super::*;

    /// Anyone who set a webhook before the list existed must find it still
    /// there. Losing it silently looks exactly like the feature breaking.
    #[test]
    fn an_old_single_webhook_becomes_a_target() {
        let stored = r#"{"discord_webhook": "https://discord.com/api/webhooks/1/x",
                         "discord_limit_mb": 50}"#;
        let mut s: Settings = serde_json::from_str(stored).expect("parses");
        s.clamp();

        assert_eq!(s.discord_targets.len(), 1);
        assert_eq!(
            s.discord_targets[0].url,
            "https://discord.com/api/webhooks/1/x"
        );
        assert_eq!(s.discord_targets[0].limit_mb, 50);
        assert!(!s.discord_targets[0].name.is_empty());
    }

    /// clamp() runs on every load, so migrating twice must not duplicate.
    #[test]
    fn migrating_twice_does_not_duplicate() {
        let stored = r#"{"discord_webhook": "https://discord.com/api/webhooks/1/x"}"#;
        let mut s: Settings = serde_json::from_str(stored).expect("parses");
        s.clamp();
        s.clamp();
        assert_eq!(s.discord_targets.len(), 1);
    }

    /// The old fields are cleared and skipped when writing, so a migrated file
    /// never carries them forward to be migrated again.
    #[test]
    fn the_old_fields_are_not_written_back() {
        let stored = r#"{"discord_webhook": "https://discord.com/api/webhooks/1/x"}"#;
        let mut s: Settings = serde_json::from_str(stored).expect("parses");
        s.clamp();

        let written = serde_json::to_string(&s).expect("serialises");
        assert!(!written.contains("discord_webhook"), "{written}");
        assert!(!written.contains("discord_limit_mb"), "{written}");
    }

    #[test]
    fn a_target_with_no_url_is_dropped() {
        let mut s = Settings {
            discord_targets: vec![
                DiscordTarget {
                    name: "Clips".into(),
                    url: "   ".into(),
                    limit_mb: 25,
                },
                DiscordTarget {
                    name: "Staff".into(),
                    url: "https://discord.com/api/webhooks/2/y".into(),
                    limit_mb: 25,
                },
            ],
            ..Default::default()
        };
        s.clamp();
        assert_eq!(s.discord_targets.len(), 1);
        assert_eq!(s.discord_targets[0].name, "Staff");
    }

    /// Each channel keeps its own ceiling: two servers can have two different
    /// boost levels, and using one number for both means refused uploads.
    #[test]
    fn limits_are_per_channel() {
        let mut s = Settings {
            discord_targets: vec![
                DiscordTarget {
                    name: "Mine".into(),
                    url: "https://discord.com/api/webhooks/1/x".into(),
                    limit_mb: 100,
                },
                DiscordTarget {
                    name: "Theirs".into(),
                    url: "https://discord.com/api/webhooks/2/y".into(),
                    limit_mb: 10,
                },
            ],
            ..Default::default()
        };
        s.clamp();
        assert_eq!(s.discord_targets[0].limit_mb, 100);
        assert_eq!(s.discord_targets[1].limit_mb, 10);
    }

    #[test]
    fn a_nonsense_limit_is_clamped() {
        let mut s = Settings {
            discord_targets: vec![DiscordTarget {
                name: "Clips".into(),
                url: "https://discord.com/api/webhooks/1/x".into(),
                limit_mb: 99_999,
            }],
            ..Default::default()
        };
        s.clamp();
        assert_eq!(s.discord_targets[0].limit_mb, 500);
    }
}

#[cfg(test)]
mod auto_session_tests {
    use super::*;

    /// Recording an entire evening without being asked is not something to opt
    /// anybody into: it is tens of gigabytes.
    #[test]
    fn recording_whole_sessions_is_off_unless_asked_for() {
        assert!(!Settings::default().auto_session);
    }

    /// Roughly what an hour costs, so the setting's copy can say so honestly.
    #[test]
    fn an_hour_at_the_default_bitrate_is_about_thirteen_gigabytes() {
        let s = Settings::default();
        let bytes_per_hour = s.bitrate_kbps as u64 * 1000 / 8 * 3600;
        assert!(
            (12..=15).contains(&(bytes_per_hour / 1_000_000_000)),
            "{} GB/hour",
            bytes_per_hour / 1_000_000_000
        );
    }
}
