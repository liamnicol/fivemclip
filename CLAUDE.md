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
ui/                 index.html + region.html + editor.html, one JS file each
```

## Conventions that are load-bearing

**Comments explain why, not what.** Several of them record a bug that has
already been fixed once; deleting them invites it back.

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

**The region overlay is built hidden and shown by `region_ready`.** A webview
paints white before its first frame, and a fullscreen white flash over a dark
game is the most visible thing this app does. The page calls `region_ready`
only after `img.decode()` resolves, so the window appears with the frozen screen
already on it - on reuse that also stops the *previous* capture flashing up,
since the old image stays on screen until the new one decodes. `region_ready` is
called on the error path too, or a failure is an invisible window and a hotkey
that looks dead, and `watch_for_a_stuck_overlay` shows it anyway after 1.5s if
the page never reports at all.

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

`ui/` is plain HTML, so it renders in any browser with `window.__TAURI__`
stubbed. This caught the blur bug above. Serve `ui/` over HTTP - `file://`
blocks image loads - and drive it with Playwright.

The server must honour Range requests. `python -m http.server` does not, so a
`<video>` reports `seekable=[0,0]`, every seek silently does nothing, and the
trim window looks broken when it is not.

`crates/capture/src/trim.rs` has tests that run a real ffmpeg. They skip unless
`FIVEMCLIP_TEST_FFMPEG` and `FIVEMCLIP_TEST_CLIP` are set, so CI stays green
without one.

## Diagnostics

`src-tauri/src/diagnostics.rs` writes `fivemclip.log` beside the settings file,
flushed per line. `diagnostics::span()` logs begin/end pairs: a `begin` with no
`end` is a hang, and names where.

Devtools are enabled in release builds. Right-click any window, Inspect.

## Releasing

Bump the version in **both** `Cargo.toml` and `src-tauri/tauri.conf.json` -
checks fail if they disagree - then tag `vX.Y.Z` and push the tag. CI builds,
signs, writes `latest.json` and publishes the release.

Add an entry at the top of `whatsnew::RELEASES` for the new version. A test
fails without one, because a release that ships an empty "what's new" splash is
worse than one that ships none.

`TAURI_SIGNING_PRIVATE_KEY` must stay set as a repository secret. Losing it
means no installed copy can ever be updated again; they would all need a manual
reinstall.

## Past bugs worth knowing

`docs/open-bug-editor.md` records a deadlock that cost four failed fixes. Read
it before changing how windows are created.
