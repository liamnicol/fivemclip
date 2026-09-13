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
        version: "0.2.13",
        lines: &[
            "Logs now survive restarts. The last five runs are kept as fivemclip.log.1 to .5, because only one was kept before - and installing an update restarts the app twice, which was enough to lose the log of the crash you were trying to diagnose.",
            "Every log now opens by saying whether the previous run shut down cleanly or died, so \"did it crash?\" is answered on the first line rather than worked out.",
            "It also records which encoder, monitor and bitrate were in use. A crash that leaves no error behind is usually a driver or a GPU encoder, and that line is the first clue. No webhooks or keys are ever written to it.",
        ],
    },
    Release {
        version: "0.2.12",
        lines: &[
            "Region capture works again after the first one. Every capture but the first opened onto \"The captured frame is missing\" - the screen was only being frozen on the path that built the overlay, and the capture before it had deleted the frame. Broken since 0.2.7; sorry.",
            "No more white flash. The overlay is built hidden and only shown once the frozen screen is actually painted onto it, so it no longer appears white over the game and fill in afterwards.",
            "This panel now fits the window. Catching up on three releases at once made it taller than the app, and the top of it could not be scrolled to at all - so it began above the top edge and the button was below the bottom one.",
            "The heading and the button stay put now while the notes scroll between them, at every window size down to the smallest the app allows.",
            "The first-run screen had the same fault, where it would have meant not being able to reach Get started.",
        ],
    },
    Release {
        version: "0.2.11",
        lines: &[
            "Upload clips to your own cloud storage, with no Discord size limit in the way. Settings \u{2192} Your own cloud storage.",
            "It is your bucket, not ours: your storage, your bill, your keys. Cloudflare R2 is the one to use - it charges nothing to serve what you upload, which is where the cost of sharing clips actually is. Pennies a month for a folder of clips.",
            "Works with anything S3-compatible. A \"To cloud\" button appears on every clip and screenshot once it is set up, and copies the link when it is done.",
        ],
    },
    Release {
        version: "0.2.10",
        lines: &[
            "A Check for updates button in Settings, which tells you what it found - including when you are already up to date, when GitHub could not be reached, and when a copy cannot update itself at all.",
            "The update banner now shows whatever tab you are on. It was inside the Record view, so it was invisible from Library and Settings.",
            "Between them, that was the whole of \"update checking does not work\": four different outcomes all showed nothing, on a tab you were probably not looking at.",
        ],
    },
    Release {
        version: "0.2.9",
        lines: &[
            "Black out the chat box in trimmed clips, for servers whose rules do not allow showing staff chat, reports or OOC. Settings \u{2192} Hiding the chat, then drag over the chat box while it has something in it.",
            "Filled solid, not blurred. A blur over moving footage can be averaged back out across frames, so a blurred clip is not the same as a clean one.",
            "It needs a re-encode, so it cannot be combined with a fast trim - the Fast button says so rather than quietly handing back a clip that still shows the chat.",
        ],
    },
    Release {
        version: "0.2.8",
        lines: &[
            "More than one Discord channel. Name them - Clips, Staff, whatever - and pick where each thing goes when you send it.",
            "Each channel keeps its own upload limit, because two servers can have two different boost levels and one number for both means refused uploads.",
            "A channel is only offered for files it would actually accept, and the trimmer squeezes to whichever channel you picked. The webhook you already had is carried over as your first channel.",
        ],
    },
    Release {
        version: "0.2.7",
        lines: &[
            "The region overlay is reused rather than rebuilt for every capture. Building a fullscreen window each time raced with the previous one closing, and it is the leading suspect for a crash after a long evening of captures.",
            "A region capture could show you the screen as it was last time. The frozen frame is always written to the same file and the overlay only ever loaded it once.",
            "The log now says when the app shut down cleanly, so a log that simply stops can be told apart from one that ends.",
        ],
    },
    Release {
        version: "0.2.6",
        lines: &[
            "FiveMClip can record your whole session: it starts when the game launches and saves when it closes, so an evening is on disk rather than only the moments you pressed a key for.",
            "Off by default, in Settings under Behaviour. It is large - the setting tells you how many gigabytes an hour at your current bitrate.",
            "A game that crashes saves the session rather than losing it, because from here a crash looks the same as closing.",
        ],
    },
    Release {
        version: "0.2.5",
        lines: &[
            "Settings apply as you change them. The Save settings button is gone, and so is changing something, walking away and finding it never took.",
        ],
    },
    Release {
        version: "0.2.4",
        lines: &[
            "Saving a clip from the tray menu respects your clip length. It was still writing the whole buffer - the hotkey and the button were fixed when the two settings were split apart, and this third way in was missed.",
            "The log names its threads and times the screen grab behind a region capture, so a slow or flashing capture says which part was slow.",
        ],
    },
    Release {
        version: "0.2.3",
        lines: &[
            "Send a clip or screenshot straight to Discord. Paste a webhook URL in Settings and pick the upload limit your server actually allows.",
            "Clips are almost always too big for Discord, so the trimmer squeezes rather than cuts: pick the moment you want and Fit & send works out the bitrate that fits it under the limit.",
            "It says what that will cost before you commit, and refuses outright when a clip is long enough that fitting it would only produce a smear.",
        ],
    },
    Release {
        version: "0.2.2",
        lines: &[
            "Taking a screenshot no longer freezes the game for about a second. It was asking the capture source for one frame per second, so it sat waiting out the whole interval before it got one.",
            "Opening FiveMClip twice now raises the copy you already have instead of starting a second one. Two copies shared a buffer folder and wrote over each other's footage.",
            "Updates are rechecked while the app is running, not only when it starts - useful if you leave it in the tray for weeks.",
            "New installs start with Windows by default, ticked on the first-run screen with the reason. A replay buffer only helps if it was already running. Anyone who turned it off stays off.",
            "How long a screen grab took is now in the log, so if your machine still hitches there is a number rather than a guess.",
        ],
    },
    Release {
        version: "0.2.0",
        lines: &[
            "Mark a moment while you record. Press F7 during a session and the Library and the trimmer both remember where it was.",
            "The trim timeline shows every mark as a tick you can click to jump to, with buttons to step between them.",
            "A three hour session is no longer something you have to scrub through to find the one thing you kept it for.",
        ],
    },
    Release {
        version: "0.1.9",
        lines: &[
            "Clip length is its own setting. A clip used to be the whole buffer, so a 20 minute buffer wrote a 20 minute file every single time you pressed the key. Keep a long buffer and a short clip.",
            "Library cards have a Folder button that shows the file in Explorer.",
            "The Record tab shows all five hotkeys, including the session and buffer ones it had been leaving out.",
        ],
    },
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
