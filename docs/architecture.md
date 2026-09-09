# Architecture

Notes on how FiveMClip is put together, and the two or three decisions that are
not obvious from reading the code.

## Project layout

```
crates/capture/     Screen capture, replay buffer, audio, screenshots
  ffmpeg.rs         Encoder pipeline candidates and the runtime probe
  ring.rs           Segment ring buffer and clip extraction
  audio.rs          WASAPI loopback and microphone capture
  sysprobe.rs       Monitor enumeration, FiveM detection
src-tauri/          Desktop app: commands, hotkeys, tray, uploads
ui/                 Front end — plain HTML/CSS/JS, no build step
tools/              Icon generator, ffmpeg fetcher
```

The capture crate is deliberately separate from the app so its Windows-specific
code can be type-checked from any platform:

```bash
rustup target add x86_64-pc-windows-gnu
cargo check --target x86_64-pc-windows-gnu -p fivemclip-capture
```

## How the replay buffer works

ffmpeg writes a continuous stream of two-second MPEG-TS segments into a ring
directory, recycling filenames via `-segment_wrap`. Saving a clip picks the
newest segments by modification time and concatenates them with `-c copy`.

MPEG-TS rather than MP4 is the important detail: TS survives being read while it
is still being written, so the seconds that matter most — the ones still in
flight when you hit the key — are recoverable.

Audio needs care too. A WASAPI loopback client returns *no frames at all* while
nothing is playing, so `audio.rs` tracks how far behind real time the stream has
fallen and injects silence to close the gap. Without that, a quiet minute would
shorten the audio track by a minute and desync everything after it.

## Why the encoder is chosen at runtime

Which capture and encode path works depends on the GPU, the driver, the ffmpeg
build and sometimes the session type - Desktop Duplication is blocked outright
in some RDP and virtual-machine setups. None of that is knowable at build time,
and guessing wrong means a user's clips are either broken or silently recorded
with a CPU encoder that costs them frames in-game.

So `ffmpeg.rs` defines eight candidate pipelines in preference order - NVENC
zero-copy, NVENC via CUDA, AMF, Quick Sync, then software x264, with a GDI
capture fallback beneath the Desktop Duplication ones. On first launch each is
run for real against a half-second null output, and the first that exits
cleanly is cached.

The cache is keyed on a fingerprint of the ffmpeg version, the display adapter
names and the selected monitor, so swapping a GPU or updating ffmpeg forces a
fresh probe rather than silently reusing a choice that no longer applies. The
Record tab exposes a "Re-test this PC" button that reports what every candidate
said, which is the first thing to ask for when someone reports bad performance.

## Two details that are easy to get wrong

**MPEG-TS, not MP4, in the ring buffer.** TS tolerates being read while it is
still being written, so the segment in flight when the user presses the hotkey
is still recoverable. That is the difference between a clip that ends at the
moment of the crash and one that ends two seconds before it.

**WASAPI loopback goes silent, not quiet.** When nothing is playing, a loopback
client returns *no frames at all* rather than frames of silence. Left alone, a
quiet minute would shorten the audio track by a minute and desync everything
after it. `audio.rs` tracks how far the stream has fallen behind wall clock and
injects silence to close the gap.

## Why the bundled ffmpeg is pinned to a release branch

`tools/fetch-ffmpeg.ps1` deliberately tracks BtbN's `n8.1` branch rather than
`master`, and that is not a stability preference - it is a hard compatibility
constraint.

BtbN's master and n9.0 builds compile against NVENC SDK 13.1, which refuses to
open the encoder unless the GPU driver is version 610 or newer:

```
Driver does not support the required nvenc API version. Required: 13.1 Found: 13.0
The minimum required Nvidia driver for nvenc is 610.00 or newer
```

A GTX 1080 Ti on a current driver reports 13.0. It fails the check, the encoder
probe falls through to software x264, and the user loses frames in the game the
tool exists to record - with no error, because everything technically worked.

Handing a community a recorder that quietly costs them performance unless they
first update their GPU driver is not a trade worth making, so the bundle tracks
the newest branch that works on ordinary drivers.

`tools/test-ffmpeg-builds.ps1` is how that was determined: it downloads each
candidate and runs a real encode. Re-run it on the oldest driver worth
supporting before changing the pinned branch. The fetch script also asserts the
version it downloaded matches the branch it asked for, so if that URL ever
starts serving something else the build fails loudly instead of shipping a
silent regression.
