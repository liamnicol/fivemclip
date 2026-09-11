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

**Hotkeys are `event.code`, not `event.key`.** global-hotkey names keys the way
`code` does - `KeyK`, `Numpad5`, `Space`, `BracketLeft`. `key` gives the
character produced, so Numpad5 arrived as `Clear`, Space as `" "` and Shift+1 as
`Shift+!`, none of which parse. Those saved fine and then never registered. The
input shows a readable label and carries the accelerator in `dataset.combo`;
`collectSettings` reads the dataset, never the value.

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
