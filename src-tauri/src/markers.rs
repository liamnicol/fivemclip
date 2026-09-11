//! Moments marked during a session.
//!
//! Same shape as the link index next door, and for the same reason: this is
//! metadata we want to edit and read back long after the file was written, and
//! a video container is not the place to keep that. Keyed by file name so
//! moving the output folder does not orphan it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;

pub struct Markers {
    path: PathBuf,
    entries: Mutex<HashMap<String, Vec<f64>>>,
}

impl Markers {
    pub fn load(path: PathBuf) -> Self {
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<HashMap<String, Vec<f64>>>(&raw).ok())
            .unwrap_or_default();
        Markers {
            path,
            entries: Mutex::new(entries),
        }
    }

    pub fn get(&self, file: &Path) -> Vec<f64> {
        key(file)
            .and_then(|k| self.entries.lock().get(&k).cloned())
            .unwrap_or_default()
    }

    /// Failing to write the index must not fail the save it belongs to - the
    /// recording is the thing that matters and it is already on disk.
    pub fn set(&self, file: &Path, mut markers: Vec<f64>) {
        let Some(key) = key(file) else { return };
        if markers.is_empty() {
            return;
        }
        markers.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        self.entries.lock().insert(key, markers);
        if let Err(e) = self.save() {
            crate::diagnostics::log(format!("could not save the marker index: {e}"));
        }
    }

    pub fn forget(&self, file: &Path) {
        let Some(key) = key(file) else { return };
        if self.entries.lock().remove(&key).is_none() {
            return;
        }
        if let Err(e) = self.save() {
            crate::diagnostics::log(format!("could not save the marker index: {e}"));
        }
    }

    fn save(&self) -> Result<(), String> {
        let snapshot = self.entries.lock().clone();
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(&snapshot).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, json).map_err(|e| e.to_string())
    }
}

fn key(file: &Path) -> Option<String> {
    file.file_name().map(|n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("fivemclip-markers-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("markers.json")
    }

    #[test]
    fn markers_survive_a_reload() {
        let path = scratch("reload");
        let m = Markers::load(path.clone());
        m.set(Path::new("/sessions/Night.mp4"), vec![12.0, 400.5]);

        let again = Markers::load(path);
        assert_eq!(
            again.get(Path::new("/sessions/Night.mp4")),
            vec![12.0, 400.5]
        );
    }

    /// The trimmer draws these straight onto a timeline, so out-of-order input
    /// would put a tick behind the one that follows it.
    #[test]
    fn markers_are_stored_in_order() {
        let m = Markers::load(scratch("order"));
        m.set(Path::new("Night.mp4"), vec![90.0, 12.0, 45.0]);
        assert_eq!(m.get(Path::new("Night.mp4")), vec![12.0, 45.0, 90.0]);
    }

    #[test]
    fn a_file_with_no_markers_reads_as_empty_not_missing() {
        let m = Markers::load(scratch("empty"));
        assert!(m.get(Path::new("Nothing.mp4")).is_empty());
    }

    #[test]
    fn deleting_the_recording_forgets_its_markers() {
        let m = Markers::load(scratch("forget"));
        m.set(Path::new("Night.mp4"), vec![1.0]);
        m.forget(Path::new("Night.mp4"));
        assert!(m.get(Path::new("Night.mp4")).is_empty());
    }
}
