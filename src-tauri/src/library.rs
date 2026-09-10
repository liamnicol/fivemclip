use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use fivemclip_capture::config::Settings;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MediaItem {
    pub path: String,
    pub name: String,
    /// "clip" or "screenshot" - drives which player the UI uses.
    pub kind: &'static str,
    pub size_bytes: u64,
    pub modified_ms: i64,
}

pub fn list(settings: &Settings) -> Vec<MediaItem> {
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
    list(settings).iter().map(|i| i.size_bytes).sum()
}

pub fn managed_dirs(settings: &Settings) -> Vec<PathBuf> {
    vec![
        settings.output_dir.clone(),
        settings.clips_dir(),
        settings.screenshots_dir(),
        settings.sessions_dir(),
    ]
}
