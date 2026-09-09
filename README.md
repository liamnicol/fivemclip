# FiveMClip

A local-first clip recorder for FiveM. Keeps the last few minutes of gameplay in
a rolling buffer, saves it to your own drive when you hit a key, takes
screenshots, and shares them without an account, a watermark, or a subscription.

Built for handing out to a community: one installer, no login, nothing phones
home.

---

## What it does

- **Replay buffer.** Recording runs continuously in the background. Press the
  hotkey and the last N seconds are written out — no re-encoding, so it lands in
  about a second even for a five minute buffer.
- **Screenshots** on a separate hotkey, PNG or JPEG.
- **Hardware encoding** via NVENC, AMD AMF or Intel Quick Sync, captured with
  DXGI Desktop Duplication. The app tests every combination on first launch and
  keeps whichever actually works on that PC.
- **Game and mic audio**, mixed with independent volume controls.
- **ImgBB upload** for screenshots, with the link copied to your clipboard.
- **YouTube hand-off** for clips (see below).
- **Tray resident.** Closing the window keeps recording. Optionally only records
  while FiveM is actually running, so it is not burning your GPU on the desktop.

Everything is written to `%USERPROFILE%\Videos\FiveMClip` by default and stays
there.

## Install

Grab the installer from the [Releases](../../releases) page and run it. It
installs per-user, so there is no UAC prompt.

Windows SmartScreen will warn about an unknown publisher until the build is
signed with a code-signing certificate — see [Code signing](#code-signing).

### Default hotkeys

| Action | Key |
| --- | --- |
| Save clip | `F9` |
| Screenshot | `F10` |
| Start/stop buffer | `Ctrl+F9` |

All three are rebindable in Settings.

## Sharing

### ImgBB (screenshots)

Get a free API key from [api.imgbb.com](https://api.imgbb.com/) and paste it into
Settings. Turn on *Upload every screenshot automatically* and every screenshot
lands on your clipboard as a link, ready to paste into Discord.

The key is yours and is stored only on your PC. FiveMClip deliberately does not
ship a shared key — a key baked into a distributed binary gets extracted and
rate-limited within a week, and then it stops working for everybody.

### YouTube (clips)

The **To YouTube** button opens YouTube's upload page, reveals the clip in
Explorer, and puts its full path on your clipboard so you can paste it straight
into the file picker.

This is a deliberate choice rather than a missing feature. Uploading through the
YouTube Data API would mean:

- **1,600 quota units per upload**, against a default project quota of 10,000 per
  day. That is six uploads per day shared across *everyone* using the app, not
  per person.
- **Every video forced to private** until the OAuth client passes Google's
  YouTube API audit, which needs a published privacy policy, a homepage, a demo
  video, and several weeks of review.

Two clicks beats a broken upload button. If you later want true one-click
uploads, the options are to complete Google's audit for a single shared client,
or to have each user create their own Google Cloud project and paste in a client
ID.

## Is this safe to use with FiveM?

It captures the screen through the Windows Desktop Duplication API and reads
audio through WASAPI loopback. It does not hook, inject into, or read the memory
of the game process, and the window is an ordinary desktop window rather than an
in-game overlay. That is the same approach OBS uses for display capture.

## Building from source

You need [Rust](https://rustup.rs/) and Windows 10 or 11.

```powershell
# Fetch the ffmpeg build that gets bundled (~80 MB, not committed)
./tools/fetch-ffmpeg.ps1

cargo install tauri-cli --version "^2" --locked
cargo tauri build
```

The installer is written to `target/release/bundle/nsis/`.

For a dev loop with hot reload of the UI:

```powershell
cargo tauri dev
```

CI builds the installer on every push and attaches it to the run. Pushing a tag
matching `v*` publishes a GitHub release.

### Project layout

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

### How the replay buffer works

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

## Code signing

Unsigned installers get a SmartScreen warning that scares off a chunk of any
community. To fix it, buy an OV or EV code-signing certificate (around $200–400
a year), then add the signing step to `.github/workflows/build.yml` with the
certificate in repository secrets. Tauri supports this natively via
`bundle.windows.certificateThumbprint`.

## Troubleshooting

**"No screen capture method worked on this PC."** Hit *Re-test this PC* on the
Record tab — it lists exactly what each method reported. Desktop Duplication is
blocked in some RDP and virtual-machine sessions.

**Recording, but the clip has no audio.** The Record tab shows a warning naming
the reason. Usually there is no default playback device, or another app has the
device open in exclusive mode.

**Clip captures the wrong monitor.** Desktop Duplication numbers its outputs
independently of Windows' display numbering. Use the *Test* button next to the
monitor picker — it saves a screenshot from whichever monitor is selected.

**Hotkey does nothing.** Something else has it registered globally. Any failure
is reported as a Windows notification when settings are saved; pick another key.

## Licence

FiveMClip is MIT licensed — see [LICENSE](LICENSE).

It bundles an unmodified ffmpeg binary, which is GPL licensed and carries its own
obligations. See [THIRD-PARTY.md](THIRD-PARTY.md) before redistributing.
