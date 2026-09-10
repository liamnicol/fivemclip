//! Where an uploaded screenshot ended up.
//!
//! ImgBB hands the link back exactly once. There is no "list my uploads" for an
//! anonymous key, so a link not written down at the moment of upload is gone as
//! soon as the clipboard is overwritten - which, for anyone taking a second
//! screenshot, is about ten seconds later.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::upload::ImgbbResult;

/// Old entries are dropped past this many. Someone who has uploaded thousands
/// of screenshots does not need the first thousand kept forever, and the file
/// is read and rewritten whole.
const MAX_ENTRIES: usize = 2000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkRecord {
    pub url: String,
    pub display_url: String,
    pub delete_url: String,
    pub uploaded_ms: i64,
}

/// Keyed by file name rather than full path, so moving the output folder in
/// Settings does not orphan every link the user has.
pub struct Links {
    path: PathBuf,
    entries: Mutex<HashMap<String, LinkRecord>>,
}

impl Links {
    pub fn load(path: PathBuf) -> Self {
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<HashMap<String, LinkRecord>>(&raw).ok())
            .unwrap_or_default();
        Links {
            path,
            entries: Mutex::new(entries),
        }
    }

    pub fn get(&self, file: &Path) -> Option<LinkRecord> {
        self.entries.lock().get(&key(file)?).cloned()
    }

    /// Failing to write the index must never fail the upload it belongs to -
    /// the user has their link either way.
    pub fn record(&self, file: &Path, result: &ImgbbResult) {
        let Some(key) = key(file) else { return };
        {
            let mut entries = self.entries.lock();
            entries.insert(
                key,
                LinkRecord {
                    url: result.url.clone(),
                    display_url: result.display_url.clone(),
                    delete_url: result.delete_url.clone(),
                    uploaded_ms: now_ms(),
                },
            );
            prune(&mut entries);
        }
        if let Err(e) = self.save() {
            crate::diagnostics::log(format!("could not save the link index: {e}"));
        }
    }

    pub fn forget(&self, file: &Path) {
        let Some(key) = key(file) else { return };
        if self.entries.lock().remove(&key).is_none() {
            return;
        }
        if let Err(e) = self.save() {
            crate::diagnostics::log(format!("could not save the link index: {e}"));
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

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn prune(entries: &mut HashMap<String, LinkRecord>) {
    if entries.len() <= MAX_ENTRIES {
        return;
    }
    let mut by_age: Vec<(String, i64)> = entries
        .iter()
        .map(|(k, v)| (k.clone(), v.uploaded_ms))
        .collect();
    by_age.sort_by_key(|(_, ms)| *ms);
    for (k, _) in by_age.iter().take(entries.len() - MAX_ENTRIES) {
        entries.remove(k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(ms: i64) -> LinkRecord {
        LinkRecord {
            url: "u".into(),
            display_url: "d".into(),
            delete_url: "x".into(),
            uploaded_ms: ms,
        }
    }

    #[test]
    fn pruning_keeps_the_newest() {
        let mut entries = HashMap::new();
        for i in 0..(MAX_ENTRIES as i64 + 10) {
            entries.insert(format!("shot-{i}.png"), record(i));
        }
        prune(&mut entries);
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert!(!entries.contains_key("shot-0.png"));
        assert!(entries.contains_key(&format!("shot-{}.png", MAX_ENTRIES + 9)));
    }

    #[test]
    fn a_small_index_is_left_alone() {
        let mut entries = HashMap::new();
        entries.insert("only.png".to_string(), record(1));
        prune(&mut entries);
        assert_eq!(entries.len(), 1);
    }
}
