# FiveMClip

Local-first clip recorder for FiveM. Tauri 2 shell, Rust capture core, plain
HTML/CSS/JS front end with no build step.

## Building

Windows only for a real run. `tools/fetch-ffmpeg.ps1` must run first - the
bundle resource is required by `tauri-build`, so nothing compiles without it.

```powershell
./tools/fetch-ffmpeg.ps1
cargo tauri dev     # or: cargo tauri build
```

The toolchain is pinned in `rust-toolchain.toml`. Do not bump it casually; CI
uses the same file and an unpinned clippy has broken the build before.

Before pushing, all three must pass:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Layout

```
crates/capture/     Screen capture, replay buffer, audio, screenshots
  ffmpeg.rs         Encoder pipeline candidates, the runtime probe, explain()
  ring.rs           Segment ring buffer, clip and session extraction
  audio.rs          WASAPI loopback and microphone capture
  disk.rs           Free-space floor and library pruning
  reaper.rs         Job object so ffmpeg dies with the app
  sysprobe.rs       Monitor enumeration, trigger-process matching
src-tauri/src/      Commands, hotkeys, tray, uploads, diagnostics
ui/                 index.html + region.html + editor.html + trim.html,
                    one JS file each
tools/harness/      Drives ui/ in Chromium with a stubbed Tauri backend
```

## Conventions that are load-bearing

**Comments explain why, not what.** Several of them record a bug that has
already been fixed once; deleting them invites it back.

**A sync command must never wait on the recorder's mutex.** Stopping a session
holds it for as long as ffmpeg takes to stitch - 80 seconds on an eleven gigabyte
one - and `get_status`, which the UI polls, is sync and so runs on the main
thread. It blocked, the message loop stopped, and Windows painted the whole
window "Not responding" while the app was working perfectly. It uses `try_lock`
now and falls back to `state.last_status`, which is closer to the truth than the
"nothing is recording" it would otherwise flicker to. `AppState::while_busy`
sets a message that is readable *without* that mutex - that is the whole reason
it does not live behind it - and the front end shows it.

**Anything touching ffmpeg goes in an async command.** Sync Tauri commands run
on the main thread, and blocking it froze the whole app - including the tray
Quit item, making the process unkillable except from Task Manager.

**Only the `main` window hides on close.** The close-to-tray handler is scoped
by label. Applied to every window it trapped the editor and the region overlay,
which then could not be closed at all.

**`WebviewUrl::App` takes a path, not a URL.** A query string becomes part of
the filename and the window renders blank. Pass data through state instead.

**Never build a window from the main thread.** `build()` waits for the event
loop, so calling it on the event loop thread deadlocks: blank window, frozen
app, unkillable except from Task Manager.

Marking the command `async` does **not** fix this - Tauri polls the future
inline, and an async fn runs synchronously up to its first await, so the build
still lands on `main`. Spawn an explicit thread. None of it reproduces on
WebKitGTK.

**Redaction defaults to a solid fill.** Pixelate and blur are reversible for
text. A blur bug once made redacted text *more* legible than the original -
verify visually, not by reading the code.

**Video redaction is a solid fill; a blur is not enough.** The editor may blur a
screenshot because there is only one frame to average. Over moving footage the
text is static while the encoder's noise is not, so averaging frames pulls a
legible edge back out of a blur. `trim::chat_box_filter` draws a filled
`drawbox`, and `trim::ffmpeg_tests::hiding_the_chat_really_removes_it` measures
the result - with a control on the source, so it cannot pass by the region being
dark to begin with.

**A percentage height only resolves against a definite one.** `#stage` was a
grid with `place-items: center` and the video capped at `max-height: 100%`. A
grid row is auto-sized, so that 100% resolved against a track whose height
depended on the video - circular, which CSS resolves as no maximum at all. The
video took its intrinsic height, overflowed a stage 150px shorter than it, and
painted over the transport row: Play, Start here and End here were unclickable
at every window size. It is flex with `min-height: 0` now, plus
`overflow: hidden` so nothing in the stage can ever cover the controls again.
The same family as the `align-items: center` note further down. Checked by
hit-testing in `tools/harness/check.js`, which is the only thing that would have
caught it.

**A hotkey on a bare key is not shared with the game.** `RegisterHotKey` does
not suppress raw input, which is how a game reads the keyboard, so FiveM still
sees the key and it looks like both work. The moment something in the game reads
the keyboard the ordinary way - the F8 console, once it is open and taking typed
input - the hotkey wins and the key stops arriving. F8 opened the console and
would not close it. `config::FIVEM_KEYS` lists the keys this applies to,
`clashes_with_fivem` tests a combo, and `migrate_hotkeys` moves a bare one onto
`Ctrl+` the same key, because changing the defaults alone fixes nothing for
anyone who has already run the app. The front end warns rather than refuses: it
is the user's keyboard.

**A recording is assembled beside its destination and renamed into place.**
`concat_segments` wrote straight to the final path, so a crash, a forced quit or
the updater part way through left a truncated file in the library - listed,
thumbnailed, playing up to the moment of death. `trim()` had always used a
scratch file; this did not. Scratch names are dotted, the library skips dotted
names, and `sweep_scratch` clears leftovers at startup.

**`find_ffmpeg` picks the first file by that name, not the first working one.**
App folder, beside the exe, then PATH - and `is_file()` is the whole test, so a
truncated download, a stub or an HTML error page saved under the name all get
handed to CreateProcess. Windows answers ERROR_BAD_EXE_FORMAT, os error 193,
"%1 is not a valid Win32 application", and that used to reach the user verbatim
out of whatever operation happened to run ffmpeg first - naming neither the file
nor which of the three places it came from. `ffmpeg::spawn_error` turns it into
something actionable and every spawn site goes through it; `ffmpeg::identify`
runs `-version` at startup so the log always says which binary was chosen and
whether it works.

**A durationless recording is not an unreadable one.** `video.duration` comes
back `Infinity` for a file whose header never got a duration written. Seeking
past the end makes the browser go and find it. The trimmer used to give up on
the spot, which made it a dead end for exactly the interrupted sessions people
most want to salvage.

**One function decides what is disabled, and says why.** `paint()` and the chat
toggle both wrote `fast.disabled` and disagreed; `paint()` runs on every drag,
so turning on Hide chat disabled Fast and the next nudge of a handle quietly
re-enabled it. `refusal()` is the single answer for Save, Save a copy and Fast,
and its text is shown in `#why` - a button that is grey for no stated reason is
indistinguishable from a broken one, which is what "the trimmer will not save"
turned out to be.

**A whole-clip trim is not always a no-op.** Save was disabled whenever the
selection covered everything, on the reasoning that writing a clip over itself
changes nothing. It does when `hide_chat` is on: that re-encodes every frame
with the chat painted out, which is the commonest reason to open the trimmer at
all. Fast stays refused there, because a stream copy of the whole file really is
a copy.

**A window closing is not a success message.** The trimmer's only sign that a
trim had worked was the window disappearing, so anything that stopped it
closing left the bar reading "Finishing the file…" over a clip that was already
written and already listed in the Library. `done()` shows "Saved <name>" first,
then closes, and if the close does not take it says the window can be closed by
hand. The log gained a matching "trim finished" line as the last thing the
command does, so a log that stops at "moved into place" says the answer never
got back to the window - a different fault from the trim failing.

**A long encode must say how far along it is.** `trim()` blocked on `.output()`
and the button just said "Trimming…", which on a multi-minute re-encode is
indistinguishable from a hang. `trim_with_progress` spawns instead, asks ffmpeg
for `-progress pipe:1 -nostats`, and reports out of ffmpeg's own numbers rather
than a timer, so the bar slows down when the encoder does. `trim()` is still
there and delegates with a no-op, which is why none of its tests had to change.

Two things that will bite whoever touches this next. **`out_time_ms` carries
microseconds**, not milliseconds - a misnamed field kept for compatibility - so
it is divided by a million like `out_time_us`; dividing by a thousand reports a
trim as finishing instantly. And **stderr has to be drained on its own thread**:
ffmpeg writes enough over a long encode to fill the pipe, and a full pipe blocks
the writer, so reading stdout to completion first waits on a process that is
waiting on us. `-nostats` matters too - without it the same progress numbers go
to stderr and push whatever went wrong out of the tail `explain` reads.

**Blackouts are a list of timed rectangles, not one region.** The chat scrolls,
so covering the line a message is on means covering a different rectangle as it
moves up the screen; covering the whole region for the whole clip to avoid that
is what made the feature unusable on footage anyone wanted to watch.
`trim::Blackout` carries a region and a time range, `chat_box_filters` builds
one `drawbox` per entry, and `Blackout::always` is the old whole-clip behaviour
expressed in the same shape rather than a branch. **The times are relative to
the trimmed output, not the source** - `-ss` has already moved the origin by the
time the filter sees a timestamp.

**The chat scanner is checked against real footage, not invented footage.**
`chatscan::real_footage_tests` runs the whole thing on a recording and skips
without `FIVEMCLIP_TEST_CHAT_CLIP` and `FIVEMCLIP_TEST_MODELS`. It has earned
this twice: a synthetic clip led to modelling the chat as a dark backing panel
it does not have, and then to colour-keying the channels, which cannot work.
It caught two more on first run - a `[Faction System]` tag over a bright window
reads as `ofP?ction System`, two characters wrong in seven, which the original
match tolerance missed; and an untagged `/me` line under a matched one inherits
its channel and gets covered. Both are pinned as tests now, the second as a
known limitation.

**Match tags loosely and only at the front of the line.** OCR of game text is
reliably wrong by a character or two - `fPaction`, `ADVERTlSING`, `RADlO`,
`IRADIO]` - so exact matching misses the lines that matter while looking like
it works. `mentions` allows one wrong character per three and searches only the
first three words, because "my faction is recruiting" said in ordinary chat is
not faction chat and blacking out that sentence would be both wrong and
baffling. The brackets are no help: OCR loses them.

**Chat channels cannot be told apart by colour.** It was the obvious approach
and it is wrong: each faction picks its own colour, so two faction lines can be
green and blue while an unrelated channel matches either. Measured on real
footage - hue separates the *tags* cleanly (`[INFO]` 60°, `[RADIO]` 38°,
`[DISPATCH]` 210°) but not the *channels*, which is the thing being asked for.
The constant is the literal word in the tag, so telling them apart means reading
the text. Luma is worse still: average brightness separates chat from no-chat on
a bright scene and not at all on a dark one, and the fraction of bright pixels
inverts between the two.

**Hiding the chat and a fast trim are mutually exclusive.** A stream copy cannot
paint over anything. `trim()` refuses the combination with the other argument
validation, before it touches the disk, and the trimmer disables Fast rather
than offering it.

**Hotkeys are `event.code`, not `event.key`.** global-hotkey names keys the way
`code` does - `KeyK`, `Numpad5`, `Space`, `BracketLeft`. `key` gives the
character produced, so Numpad5 arrived as `Clear`, Space as `" "` and Shift+1 as
`Shift+!`, none of which parse. Those saved fine and then never registered. The
input shows a readable label and carries the accelerator in `dataset.combo`;
`collectSettings` reads the dataset, never the value.

**Long-lived windows are hidden and reused, never closed and rebuilt.** `close()`
is asynchronous, so the next open can find no window through the manager while
the label is still taken. Reused windows must be told to reload: the editor
takes `editor:open`, the region overlay `region:open`. The overlay also has to
reset its `submitted` guard, or its second use is inert.

A correction to what used to be written here: a 0 ms overlay build is **not**
evidence of a failed build. `build()` returns once the window exists, without
waiting for the webview to paint, so it is genuinely that fast. A later log
showed a 0 ms build followed by reuses that found the window perfectly well.

**Everything a reused overlay needs must be prepared before the reuse branch,
not after it.** The region capture froze the screen only on the path that built
the window, so every capture after the first opened onto "The captured frame is
missing" - the one before it deleted the frame on its way out. The tell in the
log is "reusing the region overlay" with no "freezing the screen" span in front
of it.

**A hidden WebView2 window does not run its page.** Building the region overlay
with `.visible(false)` and showing it once the page reported having painted
looked like the clean way to kill the white flash. It cannot work: the page
never runs, so the message never comes, and every capture waited on the fallback
timer instead. Nothing in the browser harness can catch this - there is no
hidden-window state there and the page always runs - so any design that depends
on a Windows window state has to be tried on Windows before it is believed.

**The white flash is the webview's own background.** The overlay window is built
with `.background_color()` set dark, which is what is painted before the page
renders. The page separately keeps `#frame` hidden (`body.is-loading`) until
`img.decode()` resolves, because on reuse the *previous* capture is still on
screen until the new image decodes, and the window is already visible by then.

**The capture crate logs through the `log` facade; `diagnostics` installs the
sink.** A library that reaches into the binary's logger cannot be tested on its
own, so `crates/capture` uses `log::info!` and `diagnostics::init` installs a
`log::Log` that writes to the same file. The dependency was declared and unused
for a long time and nothing in the crate logged at all - which is how four logs
covering a whole day of recording, sessions and stitching came to contain not
one line about any of it. Anything worth asking "did that happen?" about later
gets a line.

**Settings do not restart the encoder while a session is recording.** A session
is stitched from its segments with `-c copy`, which needs every segment to have
the same streams and parameters. `apply_settings` restarted unconditionally, so
changing the encoder, resolution, fps or bitrate mid-session produced a file
that plays up to the change and is broken after it, with nothing said anywhere.
With auto-session on, a session is running most of the time, so most settings
changes landed inside one. They are stored and take effect at the next restart -
the same answer as refusing to move the output directory mid-session.

**The audio device can vanish between encoder restarts, and that breaks a
session too.** Same `-c copy` requirement: segments with an audio stream do not
concatenate onto segments without one. `Session::audio` remembers what the first
segment had and `audio_changed` records a disagreement, which is logged when it
happens and again when the session is saved.

**An update install is not a crash.** The installer kills the app without a
clean exit, so `previous_end` looked at the log and reported a crash every single
time anyone updated - in the one line people read first when investigating a real
crash. `UPDATE_EXIT` is written at the handover, and whichever marker comes last
wins.

**Spawn threads through `diagnostics::thread`, not `std::thread::spawn`.** Every
log line carries its thread name, and a log full of "unnamed" says nothing about
which piece of work stalled.

**Clip length lives on the recorder.** `save_clip()` takes no argument. It used
to, and three callers - hotkey, button, tray item - each had to remember which
settings field to read; the tray one was still passing `buffer_seconds` long
after the other two were fixed.

**A single-frame grab must not ask for a low frame rate.** ddagrab paces to the
rate it is given, so 1 fps means waiting a full second with a Desktop
Duplication open - which the game in front of it feels as a freeze.
`shot::GRAB_FPS` is that rate and has a test.

**SigV4 signs the `Host` header, port included.** `Url::port()` is `None` for
the scheme's default port, which is exactly when HTTP omits it from the header -
so the signed host is `host` on 443 and `host:9000` otherwise. Signing the bare
hostname worked against R2 and failed against every bucket on a custom port.
Caught end to end against a server that recomputed the signature, not by
reading the code.

**The S3 signer is cross-checked against botocore, not against the spec.**
`s3::sign_tests` pins signatures that botocore's `S3SigV4Auth` produced for the
same requests; `tools/sigv4-ref.py` regenerates them. Note S3 encodes the
path **once** - the generic SigV4 rule encodes it twice, and using that here
makes every key with a space in it fail. `s3::live_tests` will do a real upload
against a real bucket when `FIVEMCLIP_TEST_S3_*` are set.

**An update check has more than two outcomes, and silence is not an answer.**
`check_for_update` returns a `Status`: up to date, available, unreachable,
portable, unsupported. It used to return `Option`, so four of those showed
nothing and "the check does not work" was indistinguishable from "there is
nothing to report". The banner still only appears for `Available`; the Settings
button reports all of them.

**Centre overlay cards with `margin: auto`, never `align-items: center`.** A
flex item centred that way and taller than its container overflows in *both*
directions, and the top cannot be scrolled to however far you try - measured at
-118px on the default window. The what's-new panel shipped like that and began
above the top edge of the app with its button below the bottom one. `.sheet-card`
and `.setup-card` both use `margin: auto`, and the card is capped at
`min(100vh - 48px, 620px)` with only its body scrolling.

**The window is 1080x720 by default and 860x580 at minimum, and it is never
maximised.** Anything that only fits a large window does not fit. Check new
overlays at 860x580 before believing them.

**The update banner is a sibling of `<main>`, not part of a view.** It lived
inside `#view-record`, so it was `display: none` from Library and Settings -
where people spend most of their time. It supplies its own gutter and centring
because it no longer inherits them.

**The updater follows GitHub's *latest release*, which is by publish date, not
version.** Publishing an older tag after a newer one repoints every installed
copy at an older manifest, which reads to the updater as "up to date" for ever.
CI refuses to publish behind the current latest.

**Print Screen only ever arrives as a `keyup`.** Windows takes the keydown for
the clipboard grab and the Snipping Tool, so a keydown-only capture silently
ignores the key. The hotkey boxes listen on both; `KEYUP_ONLY` in `ui/main.js`
names the keys that need it.

**Registering hotkeys must not happen on the main thread.** The plugin posts to
the main thread and blocks on the reply, so calling it from there waits on a
task that cannot run until the wait ends - the same trap as building a window.

**A session is MPEG-TS segments until it is stopped.** Nothing is ever written
to the session file while recording - it is assembled from the ring afterwards.
So the output container buys no crash safety, which is why sessions moved from
MKV to MP4. `.mkv` stays in the library's extension list for sessions recorded
before that; dropping it would look like the app had deleted them.

**A copy trim plays from the right frame and still contains the wrong ones.**
`-c copy` cannot start anywhere but a keyframe, but the MP4 edit list moves
playback to the exact requested time - so it *looks* frame-accurate while
physically keeping up to two seconds of what was cut, which `-ignore_editlist`
brings straight back. That is why the default re-encodes. Measured, not
assumed; `trim::ffmpeg_tests::a_fast_trim_keeps_what_it_appears_to_cut` pins it.

**The editor composes at full image size, then blits the crop.** Pixelate and
blur read pixels back out of the canvas they are painting on, so painting into
a cropped canvas reads the wrong source rectangle. Marks are stored in original
image coordinates; the crop is just another undoable mark.

## Testing the front end without Windows

`tools/harness/` does this, and `node check.js` in it is the suite. It serves
`ui/` over HTTP, installs a `window.__TAURI__` whose commands each scenario
supplies, and drives the pages with Playwright.

```
cd tools/harness && npm install && node check.js
```

Every case in `check.js` is a bug that shipped. Add to it rather than probing by
hand; the trimmer's controls were unclickable at every window size for as long
as this did not exist, and nothing in the code reads as wrong.

**Clicking is the check, not visibility.** `hits()` asks `elementFromPoint` what
a click would actually land on. The buttons that did nothing were visible,
enabled and stable the whole time - a `<video>` was painted over them.

**Fixtures are VP9/Opus WebM, and that is not a preference.** Playwright's
Chromium is built without the proprietary codecs, so an `.mp4` fixture loads as
"Could not open that clip" and every timeline test fails for a reason unrelated
to the code. WebView2 plays H.264 perfectly well. `media.js` generates them with
ffmpeg; they are gitignored.

**The server must honour Range requests.** `python -m http.server` does not, so
a `<video>` reports `seekable=[0,0]`, every seek silently does nothing, and the
trim window looks broken when it is not. `serve.js` handles them, suffix ranges
included.

**A stubbed command that is not listed rejects.** Resolving to `undefined`
instead lets a page carry on into a state the real app would never reach, which
is how a harness starts certifying the wrong behaviour.

`crates/capture/src/trim.rs` has tests that run a real ffmpeg. They skip unless
`FIVEMCLIP_TEST_FFMPEG` and `FIVEMCLIP_TEST_CLIP` are set, so CI stays green
without one.

## Diagnostics

`src-tauri/src/diagnostics.rs` writes `fivemclip.log` beside the settings file,
flushed per line. `diagnostics::span()` logs begin/end pairs: a `begin` with no
`end` is a hang, and names where.

**The last five runs are kept** as `fivemclip.log.1` to `.5`. One generation was
not enough and failed in a specific way worth remembering: installing an update
restarts the app twice, so the log of the crash being investigated was pushed
off the end before anyone looked at it.

**Each log says on its first lines whether the previous run crashed**, judged by
whether the rotated log ends with `diagnostics::CLEAN_EXIT`, and what the
capture was configured to do - encoder, monitor, fps, bitrate. A crash with no
`PANIC` line is something below Rust, and the encoder is the first suspect.
Nothing secret goes in the log: these get pasted into Discord.

Devtools are enabled in release builds. Right-click any window, Inspect.

## The OCR models

`tools/fetch-ocr-models.ps1` must run before a build that bundles them, the
same as ffmpeg - they are gitignored, and `tauri.conf.json` lists them as
resources, so the bundle step fails without them. `chatscan::Models::beside_exe`
finds them at runtime the way `find_ffmpeg` does.

## Releasing

Bump the version in **both** `Cargo.toml` and `src-tauri/tauri.conf.json` -
checks fail if they disagree - then tag `vX.Y.Z` and push the tag. CI builds,
signs, writes `latest.json` and publishes the release.

Add an entry at the top of `whatsnew::RELEASES` for the new version. A test
fails without one, because a release that ships an empty "what's new" splash is
worse than one that ships none.

**Never add to the entry of a version that has already been tagged.** It has
happened three times: work lands on top of an unreleased version number, that
number gets tagged in between, and the entry for a build already on people's
machines grows lines describing fixes it does not contain. The what's-new panel
is the one place the app explains itself, and one claiming a fix that is not
there is worse than one that says nothing. Check what is published before
editing an entry - `gh release list`, or the tags - and if the version is out,
the work belongs in a new one. `whatsnew::shipped_notes_are_immutable` compares
every entry against what its own tag shipped and fails if they have drifted; it
skips silently where the tags are not fetched, so `git fetch --tags` first for
it to mean anything.

`TAURI_SIGNING_PRIVATE_KEY` must stay set as a repository secret. Losing it
means no installed copy can ever be updated again; they would all need a manual
reinstall.

## Past bugs worth knowing

`docs/open-bug-editor.md` records a deadlock that cost four failed fixes. Read
it before changing how windows are created.
