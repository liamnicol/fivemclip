//! The "what's new" splash shown on the first launch after an update.
//!
//! The notes ship inside the binary rather than being fetched from the release
//! feed. A changelog that needs the network is a changelog that shows up blank
//! for anyone whose update landed while they were offline, and the text is
//! already written by the time the build runs.

use serde::Serialize;
use tauri::State;

use crate::state::AppState;

pub struct Release {
    pub version: &'static str,
    pub lines: &'static [&'static str],
}

/// Newest first. Add to the top when tagging a release; anything older than
/// the version a user last saw is skipped, so someone jumping three versions
/// gets all three.
pub const RELEASES: &[Release] = &[
    Release {
        version: "0.1.8",
        lines: &[
            "Print Screen can be used as a hotkey. Windows swallows that key on the way down, so the settings box never saw it being pressed at all.",
            "If Windows is holding Print Screen for the Snipping Tool, the app now says so and points at the setting instead of blaming another program.",
        ],
    },
    Release {
        version: "0.1.7",
        lines: &[
            "Rebinding a hotkey works. Numpad keys, Space and anything with Shift were saved in a form the app could not register, so those bindings silently never fired - function keys and plain letters were fine, which is what made it look like it worked.",
            "Sessions save as MP4 instead of MKV, so Windows will preview them and anywhere you upload them will take them. Sessions you already have keep working.",
            "Library cards say whether they are a clip, a session or a screenshot - three recordings of the same scene were three identical thumbnails.",
        ],
    },
    Release {
        version: "0.1.6",
        lines: &[
            "Clips and sessions can be trimmed: Trim in the Library, drag the two handles, save.",
            "Save re-encodes so the part you cut is genuinely gone. Fast trim is instant but leaves up to two seconds of it inside the file - use it for tidying a highlight, not for cutting something out.",
            "The Library refreshes itself after an edit or a trim instead of showing the old thumbnail until you press Refresh.",
        ],
    },
    Release {
        version: "0.1.5",
        lines: &[
            "The screenshot editor can mark things up as well as hide them: Arrow, Box and Crop, alongside the redaction tools.",
            "Arrows and boxes are drawn with a dark outline, so one colour stays readable on a night street and on a blown-out minimap.",
            "Crop is undoable like everything else - it is applied when you save, not when you drag it.",
            "Portable copies no longer offer an update they cannot apply. Download the new zip instead.",
        ],
    },
    Release {
        version: "0.1.4",
        lines: &[
            "Screenshots remember their ImgBB link - upload once, copy it again any time from the Library.",
            "The disk space sliders finally show their value, so \"stop recording when free space drops below\" says below what.",
            "This screen: after an update, a short note on what changed.",
        ],
    },
];

#[derive(Debug, Clone, Serialize)]
pub struct WhatsNew {
    pub version: String,
    pub releases: Vec<ReleaseNotes>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReleaseNotes {
    pub version: String,
    pub lines: Vec<String>,
}

/// Sort key for a version string, ignoring any `-dev.x` suffix so a local
/// build compares equal to the release it was cut from.
fn ordinal(version: &str) -> (u32, u32, u32) {
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let mut parts = core.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// Notes the user has not seen yet, or `None` if there is nothing to show.
#[tauri::command]
pub fn whats_new(state: State<AppState>) -> Option<WhatsNew> {
    let current = env!("CARGO_PKG_VERSION");
    let (last_seen, setup_complete) = {
        let s = state.settings.lock();
        (s.last_seen_version.clone(), s.setup_complete)
    };

    if last_seen == current {
        return None;
    }

    let releases: Vec<ReleaseNotes> = if last_seen.is_empty() {
        if !setup_complete {
            // A fresh install. First-run setup is the introduction; a list of
            // changes to software they have never run means nothing.
            return None;
        }
        // Upgraded from a version that predates this field, so there is no way
        // to know what they last ran. Show the release they just landed on.
        RELEASES
            .iter()
            .filter(|r| ordinal(r.version) == ordinal(current))
            .map(notes)
            .collect()
    } else {
        let floor = ordinal(&last_seen);
        RELEASES
            .iter()
            .filter(|r| ordinal(r.version) > floor)
            .map(notes)
            .collect()
    };

    if releases.is_empty() {
        return None;
    }
    Some(WhatsNew {
        version: current.to_string(),
        releases,
    })
}

fn notes(r: &Release) -> ReleaseNotes {
    ReleaseNotes {
        version: r.version.to_string(),
        lines: r.lines.iter().map(|l| l.to_string()).collect(),
    }
}

/// Remember that these notes have been read.
#[tauri::command]
pub fn dismiss_whats_new(state: State<AppState>) -> Result<(), String> {
    state.settings.lock().last_seen_version = env!("CARGO_PKG_VERSION").to_string();
    state.persist()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_builds_compare_equal_to_their_release() {
        assert_eq!(ordinal("0.1.4"), ordinal("0.1.4-dev.36.abc1234"));
    }

    #[test]
    fn ordering_is_numeric_not_lexical() {
        assert!(ordinal("0.1.10") > ordinal("0.1.9"));
    }

    #[test]
    fn every_release_is_listed_newest_first() {
        for pair in RELEASES.windows(2) {
            assert!(
                ordinal(pair[0].version) > ordinal(pair[1].version),
                "{} should sort above {}",
                pair[0].version,
                pair[1].version
            );
        }
    }

    /// The splash is useless if the version being shipped has no entry.
    #[test]
    fn the_current_version_has_notes() {
        let current = ordinal(env!("CARGO_PKG_VERSION"));
        assert!(
            RELEASES.iter().any(|r| ordinal(r.version) == current),
            "add a whatsnew::RELEASES entry for {}",
            env!("CARGO_PKG_VERSION")
        );
    }
}
