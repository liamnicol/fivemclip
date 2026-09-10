//! Keeping the recorder from filling somebody's drive.
//!
//! The replay buffer writes continuously and a session recording writes without
//! bound, so "there is plenty of space" is only ever true for a while. Two
//! guards: a floor the recorder refuses to cross, and an optional cap on how
//! much the library is allowed to accumulate.

use std::path::{Path, PathBuf};

use crate::config::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SpaceVerdict {
    Fine,
    /// Above the floor, but not by much - worth telling the user before the
    /// recorder stops on its own mid-session.
    Low,
    /// Below the floor. Recording must not start, and must stop if running.
    Critical,
}

/// Once stopped, require this much margin above the floor before restarting.
///
/// Without hysteresis, a drive hovering at the threshold would have the
/// recorder stopping and starting every few seconds, which is worse than
/// either state.
const RESTART_MARGIN: f64 = 1.25;
/// Warn while there is still time to do something about it.
const WARN_MARGIN: f64 = 2.0;

pub fn verdict(free: u64, settings: &Settings) -> SpaceVerdict {
    let floor = settings.min_free_bytes();
    if free < floor {
        SpaceVerdict::Critical
    } else if (free as f64) < floor as f64 * WARN_MARGIN {
        SpaceVerdict::Low
    } else {
        SpaceVerdict::Fine
    }
}

/// May recording resume after having been stopped for low disk?
pub fn may_resume(free: u64, settings: &Settings) -> bool {
    free as f64 >= settings.min_free_bytes() as f64 * RESTART_MARGIN
}

/// Free bytes on the volume holding the output folder, if it can be read.
pub fn free_for(settings: &Settings) -> Option<u64> {
    let mut probe = settings.output_dir.as_path();
    loop {
        if probe.exists() {
            return crate::sysprobe::free_space_bytes(probe);
        }
        probe = probe.parent()?;
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PruneReport {
    pub deleted: usize,
    pub freed_bytes: u64,
}

/// Delete the oldest saved media until the library fits under its cap.
///
/// Deliberately never touches the ring buffer - that is bounded by ffmpeg's own
/// segment wrap and deleting from underneath a running recorder would corrupt
/// the next clip someone saved.
pub fn prune(settings: &Settings) -> PruneReport {
    let mut report = PruneReport {
        deleted: 0,
        freed_bytes: 0,
    };
    if !settings.auto_prune {
        return report;
    }

    let mut files: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
    for dir in [
        settings.clips_dir(),
        settings.screenshots_dir(),
        settings.sessions_dir(),
    ] {
        collect(&dir, &mut files);
    }

    let total: u64 = files.iter().map(|(_, size, _)| size).sum();
    let cap = settings.max_library_bytes();
    if total <= cap {
        return report;
    }

    // Oldest first, so what goes is what the user is least likely to want.
    files.sort_by_key(|(_, _, modified)| *modified);

    let mut over = total - cap;
    for (path, size, _) in files {
        if over == 0 {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            report.deleted += 1;
            report.freed_bytes += size;
            over = over.saturating_sub(size);
        }
    }
    report
}

fn collect(dir: &Path, out: &mut Vec<(PathBuf, u64, std::time::SystemTime)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        out.push((entry.path(), meta.len(), modified));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings {
            min_free_gb: 10,
            ..Default::default()
        }
    }

    #[test]
    fn verdict_tracks_the_floor() {
        let s = settings();
        assert_eq!(verdict(50_000_000_000, &s), SpaceVerdict::Fine);
        assert_eq!(verdict(15_000_000_000, &s), SpaceVerdict::Low);
        assert_eq!(verdict(5_000_000_000, &s), SpaceVerdict::Critical);
    }

    #[test]
    fn resuming_needs_more_room_than_stopping_did() {
        let s = settings();
        // Exactly at the floor is where it stopped; resuming there would just
        // stop again a moment later.
        assert!(!may_resume(10_000_000_000, &s));
        assert!(may_resume(13_000_000_000, &s));
    }

    #[test]
    fn pruning_does_nothing_when_disabled() {
        let s = Settings {
            auto_prune: false,
            max_library_gb: 1,
            ..Default::default()
        };
        let report = prune(&s);
        assert_eq!(report.deleted, 0);
    }

    #[test]
    fn pruning_removes_oldest_first_until_under_cap() {
        let root = std::env::temp_dir().join(format!("fivemclip-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        let s = Settings {
            output_dir: root.clone(),
            auto_prune: true,
            // Cap of 1 GB against four 400 MB files: two must go.
            max_library_gb: 1,
            ..Default::default()
        };
        std::fs::create_dir_all(s.clips_dir()).unwrap();

        // Sparse files, so the test does not actually write 1.6 GB.
        for name in ["a.mp4", "b.mp4", "c.mp4", "d.mp4"] {
            let file = std::fs::File::create(s.clips_dir().join(name)).unwrap();
            file.set_len(400_000_000).unwrap();
            drop(file);
            std::thread::sleep(std::time::Duration::from_millis(30));
        }

        let report = prune(&s);
        assert_eq!(report.deleted, 2, "expected the two oldest to go");
        assert!(!s.clips_dir().join("a.mp4").exists());
        assert!(!s.clips_dir().join("b.mp4").exists());
        assert!(s.clips_dir().join("c.mp4").exists());
        assert!(s.clips_dir().join("d.mp4").exists());

        let _ = std::fs::remove_dir_all(&root);
    }
}
