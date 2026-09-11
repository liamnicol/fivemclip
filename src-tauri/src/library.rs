use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use fivemclip_capture::config::Settings;
use serde::Serialize;

use crate::links::{LinkRecord, Links};
use crate::markers::Markers;

#[derive(Debug, Clone, Serialize)]
pub struct MediaItem {
    pub path: String,
    pub name: String,
    /// "clip" or "screenshot" - drives which player the UI uses.
    pub kind: &'static str,
    pub size_bytes: u64,
    pub modified_ms: i64,
    /// Where this one was uploaded, if it ever was. The whole point of keeping
    /// the index: the link outlives the toast that first showed it.
    pub link: Option<LinkRecord>,
    /// How many moments were marked during this recording. The Library only
    /// needs the count; the trimmer asks for the offsets when it opens one.
    pub marker_count: usize,
}

pub fn list(settings: &Settings, links: &Links, markers: &Markers) -> Vec<MediaItem> {
    let mut items = scan(settings);
    for item in &mut items {
        let path = Path::new(&item.path);
        item.link = links.get(path);
        item.marker_count = markers.get(path).len();
    }
    items
}

fn scan(settings: &Settings) -> Vec<MediaItem> {
    let mut items = Vec::new();
    collect(&settings.clips_dir(), "clip", &["mp4", "mkv"], &mut items);
    collect(
        &settings.sessions_dir(),
        "session",
        &["mkv", "mp4"],
        &mut items,
    );
    collect(
        &settings.screenshots_dir(),
        "screenshot",
        &["png", "jpg", "jpeg"],
        &mut items,
    );
    items.sort_by_key(|i| std::cmp::Reverse(i.modified_ms));
    items
}

fn collect(dir: &Path, kind: &'static str, exts: &[&str], out: &mut Vec<MediaItem>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let matches_ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
            .unwrap_or(false);
        if !matches_ext {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        // A clip still being muxed shows up as a zero-byte file for a moment.
        if meta.len() == 0 {
            continue;
        }
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        out.push(MediaItem {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: path.to_string_lossy().into_owned(),
            kind,
            size_bytes: meta.len(),
            modified_ms,
            link: None,
            marker_count: 0,
        });
    }
}

/// Refuse to touch anything outside the folders we manage. The path comes from
/// the front end, and "delete this file" is not a command worth trusting.
pub fn is_managed(settings: &Settings, path: &Path) -> bool {
    let Ok(canonical) = path.canonicalize() else {
        return false;
    };
    [
        settings.clips_dir(),
        settings.screenshots_dir(),
        settings.sessions_dir(),
    ]
    .iter()
    .filter_map(|d| d.canonicalize().ok())
    .any(|d| canonical.starts_with(d))
}

pub fn total_size(settings: &Settings) -> u64 {
    scan(settings).iter().map(|i| i.size_bytes).sum()
}

pub fn managed_dirs(settings: &Settings) -> Vec<PathBuf> {
    vec![
        settings.output_dir.clone(),
        settings.clips_dir(),
        settings.screenshots_dir(),
        settings.sessions_dir(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::Links;

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("fivemclip-library-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Sessions are written as MP4 now. The ones recorded before that change
    /// are still on people's drives, and a library that quietly stopped listing
    /// them would look exactly like the app had deleted them.
    #[test]
    fn sessions_recorded_as_mkv_are_still_listed() {
        let dir = scratch("mkv");
        let settings = Settings {
            output_dir: dir.clone(),
            ..Default::default()
        };
        std::fs::create_dir_all(settings.sessions_dir()).unwrap();
        std::fs::write(settings.sessions_dir().join("Session_old.mkv"), b"x").unwrap();
        std::fs::write(settings.sessions_dir().join("Session_new.mp4"), b"x").unwrap();

        let links = Links::load(dir.join("links.json"));
        let markers = Markers::load(dir.join("markers.json"));
        let names: Vec<String> = list(&settings, &links, &markers)
            .into_iter()
            .filter(|i| i.kind == "session")
            .map(|i| i.name)
            .collect();

        assert!(names.contains(&"Session_old.mkv".to_string()), "{names:?}");
        assert!(names.contains(&"Session_new.mp4".to_string()), "{names:?}");
    }

    /// A clip and a session both being MP4 must not make one show up as the
    /// other - the folder is what decides, and the UI labels the card from it.
    #[test]
    fn kind_comes_from_the_folder_not_the_extension() {
        let dir = scratch("kind");
        let settings = Settings {
            output_dir: dir.clone(),
            ..Default::default()
        };
        std::fs::create_dir_all(settings.clips_dir()).unwrap();
        std::fs::create_dir_all(settings.sessions_dir()).unwrap();
        std::fs::write(settings.clips_dir().join("Clip_a.mp4"), b"x").unwrap();
        std::fs::write(settings.sessions_dir().join("Session_a.mp4"), b"x").unwrap();

        let links = Links::load(dir.join("links.json"));
        let markers = Markers::load(dir.join("markers.json"));
        let items = list(&settings, &links, &markers);
        let kind = |name: &str| {
            items
                .iter()
                .find(|i| i.name == name)
                .map(|i| i.kind)
                .unwrap_or("missing")
        };
        assert_eq!(kind("Clip_a.mp4"), "clip");
        assert_eq!(kind("Session_a.mp4"), "session");
    }
}
