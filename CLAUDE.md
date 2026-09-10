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

**Redaction defaults to a solid fill.** Pixelate and blur are reversible for
text. A blur bug once made redacted text *more* legible than the original -
verify visually, not by reading the code.

## Testing the front end without Windows

`ui/` is plain HTML, so it renders in any browser with `window.__TAURI__`
stubbed. This caught the blur bug above. Serve `ui/` over HTTP - `file://`
blocks image loads - and drive it with Playwright.

## Diagnostics

`src-tauri/src/diagnostics.rs` writes `fivemclip.log` beside the settings file,
flushed per line. `diagnostics::span()` logs begin/end pairs: a `begin` with no
`end` is a hang, and names where.

Devtools are enabled in release builds. Right-click any window, Inspect.

## Open bug

See `docs/open-bug-editor.md`. Not solved. Read it before touching the editor.
