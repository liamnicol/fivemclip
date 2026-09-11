# FiveMClip

A local-first clip recorder for FiveM. Keeps the last few minutes of gameplay in
a rolling buffer, saves it to your own drive when you hit a key, records whole
sessions, takes and edits screenshots, and shares them without an account, a
watermark, or a subscription.

Built for handing out to a community: one installer, no login, nothing phones
home.

## Updates

FiveMClip checks for a new release when it starts and shows a banner if one
exists. Nothing downloads until you press the button - a recorder that restarts
itself mid-session is worse than one running a week behind.

After it updates, the next launch shows a short note on what changed. Skip
several versions and you get all of them.

Updates are signed. The installed app only accepts an update whose signature
matches a key baked into it at build time, so compromising the GitHub account
is not by itself enough to push code to anyone's machine.

The portable build does not self-update and does not show the banner — the
update artefact is the installer, and running it from a portable copy would
install a second one elsewhere rather than update the folder you are using.
Download the new zip instead.

## Scope

FiveMClip is built and maintained for FiveM. It records the screen rather than a
particular game, so it works with anything you happen to be playing - but FiveM
is what gets tested, and what bugs get fixed for.

If it does not work with some other game, that is not a bug that will be chased.
It is MIT licensed and the source is right here: fork it and make it work for
your case.

---

## What it does

- **Replay buffer.** Recording runs continuously in the background. Press the
  hotkey and the last N seconds are written out — no re-encoding, so it lands in
  about a second. How much it remembers and how much a clip keeps are separate
  settings, so a 20 minute buffer does not mean a 20 minute file every time.
- **Session recording** for a whole night, started and stopped on its own
  hotkey and kept apart from your clips, with a key to mark moments as you go.
- **Screenshots** on a separate hotkey, full screen or a rectangle you drag.
- **A screenshot editor** that hides things (black out, pixelate, blur) and
  marks them up (arrow, box, crop).
- **Clip trimming** — drag two handles and keep only the part that matters.
- **Hardware encoding** via NVENC, AMD AMF or Intel Quick Sync, captured with
  DXGI Desktop Duplication. The app tests every combination on first launch and
  keeps whichever actually works on that PC.
- **Game and mic audio**, mixed with independent volume controls.
- **ImgBB upload** for screenshots, with the link copied to your clipboard and
  kept in the Library so you can copy it again weeks later.
- **YouTube hand-off** for clips (see below).
- **Disk guards** so a recorder left running cannot fill your drive.
- **Tray resident.** Closing the window keeps recording. Optionally only records
  while a game you have named is running, so it is not burning your GPU on the
  desktop.

Settings has a list of executables that mean "record now", pre-filled with
FiveM and RedM. Add anything you like, or turn the condition off and have the
buffer always running.

Everything is written to one folder you pick on first run — it suggests
`%USERPROFILE%\Videos\FiveMClip` — and stays there. `Clips`, `Sessions` and
`Screenshots` are subfolders of it. Change it later in Settings.

## Install

Grab the installer from the [Releases](../../releases) page and run it. It
installs per-user, so there is no UAC prompt.

First launch asks where clips should go, how much to remember, and whether to
start with Windows — which is ticked, because a replay buffer only helps if it
was already running when the thing worth keeping happened. Untick it if you
would rather launch it yourself.

Only one copy runs at a time. Opening it again raises the one you have, rather
than starting a second that would write over the first one's buffer.

### Windows will try to stop you

Releases are not code-signed, so Windows SmartScreen blocks the installer. This
is not a sign anything is wrong - it is what Windows shows for any application
whose publisher it does not recognise, which includes every small project that
has not paid for a certificate.

You will see this:

<img src="docs/images/smartscreenpre.png" alt="SmartScreen: Windows protected your PC, with a More info link" width="420">

Click **More info**. The dialog expands to show what is being run:

<img src="docs/images/smartscreenpost.png" alt="SmartScreen expanded, showing the application name and a Run anyway button" width="420">

Then click **Run anyway**.

"Unknown publisher" is expected - it means unsigned, not unsafe. If you would
rather check before trusting it: every release is built in public by GitHub
Actions from the tagged source in this repository, so you can read both the code
and the run that produced the file you downloaded. You can also upload the
installer to VirusTotal yourself.

### Portable

Every release also ships `FiveMClip-<version>-portable.zip`. Unzip it somewhere
you have write access and run `FiveMClip.exe` - no installer, no admin, nothing
written outside the folder.

|  | Installer | Portable |
| --- | --- | --- |
| Settings live in | your Windows profile | the program's folder |
| Recordings default to | `Videos\FiveMClip` | a folder beside the program |
| Start with Windows | yes | yes, while the folder stays put |
| WebView2 runtime | installed if missing | must already be present |
| Uninstall | Add or remove programs | delete the folder |

Portable is the better choice on a shared or locked-down PC, or to try
FiveMClip without installing anything. The installer is the better choice
otherwise, mainly because it sorts out WebView2 - which ships with Windows 11
and most Windows 10 installs, but not all.

Portable mode is switched on by the `portable.txt` file in the zip. Delete it
and that copy behaves like an installed one.

## Default hotkeys

| Action | Key |
| --- | --- |
| Save clip | `F9` |
| Screenshot | `F10` |
| Screenshot a region | `F11` |
| Mark this moment | `F7` |
| Start / stop a session | `F8` |
| Toggle the buffer | `Ctrl+F9` |

All five are rebindable in Settings → Hotkeys: click a box, press the keys you
want, then **Save settings**. `Esc` clears a binding.

Function keys, letters, digits, numpad keys, `Print Screen` and combinations
with `Ctrl`/`Shift`/`Alt` all work. If Windows refuses one because another
program already holds it, you get a notification saying so when you save.

Print Screen is worth knowing about: Windows 11 gives that key to the Snipping
Tool by default, so FiveMClip cannot have it until you turn that off in
**Settings → Accessibility → Keyboard → "Use the Print screen key to open
Snipping Tool"**. The app will tell you if that is what is in the way.

## Recording a whole session

The replay buffer answers "that was good, keep it". A session recording answers
"record the next three hours" — a whole shift, a training run, a court case.

The replay buffer has to be running first, because a session is recorded from
the same footage. Press the session hotkey to start and press it again to stop,
or use the button on the Record tab. Stopping stitches it into one MP4 in
`Sessions` — a few seconds for a long night, because there is a lot of it, but
still no re-encoding.

Two things worth knowing:

- **Your clip hotkey still works throughout.** Pulling a highlight out
  mid-session does not interrupt the recording.
- **Sessions are never auto-deleted.** The library cap below leaves them alone.
  They are the largest files the app produces and the ones nobody wants
  disappearing on them, so removing them is a decision you make yourself.

### Marking moments

Press `F7` while a session is recording and FiveMClip writes down where you
were. The session card counts them as you go, the Library shows how many a
recording has, and the trimmer draws each one on the timeline as a tick you can
click to jump straight to - with **‹ Mark** and **Mark ›** to step between them.

This is what makes a long session worth keeping. Three hours of footage with no
marks is three hours of scrubbing to find the one thing you recorded it for.

Marks are approximate rather than frame-exact: the stitched file can begin up to
two seconds after the session did. They put you within a second or two of the
moment, and the trim handles do the rest.

**Discard** throws the session away without writing it, for when you started one
by accident.

## Your library

The Library tab lists everything the app has saved, newest first, filtered by
clips, sessions or screenshots. Each card says which it is, because a clip and a
session recorded a minute apart are the same picture twice. Each one can be opened, shared or deleted; clips
and sessions can be trimmed, and screenshots edited.

Deleting from here does not go via the Recycle Bin.

## Running out of disk

A replay buffer writes continuously and a session writes without limit, so
"there is plenty of space" is only ever true for a while. Two guards, both in
Settings → Disk space:

- **A floor.** Recording stops when free space drops below it, and the Record
  tab warns you well before it gets there. Recording resumes on its own once
  there is real headroom again — not the instant it creeps back over the line,
  which would have it stopping and starting every few seconds.
- **A library cap**, off by default. When on, the oldest clips and screenshots
  are deleted once the total goes over. Sessions are never touched, and nothing
  goes to the Recycle Bin. It is off by default because quietly deleting
  somebody's recordings is not something to opt them into.

## Trimming a clip

**Trim** in the Library opens the clip with a timeline under it. Drag the two
handles, or scrub and press `I` and `O`, then save. **Play selection** plays
back exactly what you are about to keep. Arrow keys nudge a handle by a tenth
of a second, `Shift` by a whole one.

There are two ways to save it, and the difference matters more than it looks:

| | Speed | Picture | The part you cut |
| --- | --- | --- | --- |
| **Save** / **Save a copy** | a few seconds | re-encoded once | **gone** |
| **Fast trim** | instant | untouched | still in the file |

A stream copy can only start at a keyframe, and these clips carry one every two
seconds. The trimmed file says where playback should begin, so it *plays* from
exactly the right frame — but up to two seconds of what you cut is still
physically inside it, and a player that ignores that instruction will show it.

So **Fast trim** is right for tidying up a highlight, and wrong for cutting
something out. If you are trimming to remove a name, a plate or staff chat, use
**Save**. It is the same trap as pixelating a name: it looks removed and is not.

Fast trim exists because re-encoding a three hour session recording is not
something anyone is going to sit through.

## Screenshots

Two ways to take one: the whole screen, or the region hotkey, which dims the
screen and lets you drag a rectangle. Either can be copied straight to the
clipboard, uploaded to ImgBB, or both — for a region grab the clipboard copy is
usually the whole point, and the file is the backup.

JPEG by default, because a JPEG uploads instantly; switch to PNG in Settings if
you want them pixel-exact.

### Editing them

Screenshots have an **Edit** button in the Library, and a region capture can open
the editor automatically (Settings → Screenshots) — for anyone routinely hiding
names or plates, that saves a trip through the library every single time.

Pick a tool and drag on the image; `Ctrl`+`Z` undoes, and nothing is written
until you save.

### Hiding things

Drag a box over anything that should not be shared - a name, a plate, staff
chat, a warrant reference. Three tools, and the difference between them matters:

| | What it does | Safe for real details? |
| --- | --- | --- |
| **Black out** | Solid fill | **Yes** |
| Pixelate | Coarse blocks | No |
| Blur | Heavy blur | No |

**Pixelation and blur can be reversed.** Tools exist that reconstruct
pixelated text when the font is known - which for a game interface it always
is - and heavy blur is better but not a guarantee. For a real name, address,
plate or case reference, use **Black out**; it is the default for that reason.
Pixelate and blur are there for when you want to obscure something without the
screenshot looking censored.

### Marking things up

| | What it does |
| --- | --- |
| **Arrow** | Points at something. Drag from the tail to the tip. |
| **Box** | Outlines something without covering it. |
| **Crop** | Trims the image down to the rectangle you drag. |

Arrows and boxes are drawn with a dark outline so one colour stays readable on
both a night street and a white minimap. Crop is undoable like everything else -
it is applied when you save, not when you drag it.

### Saving

Saving overwrites the original, so an unredacted copy is not left sitting in
your screenshots folder. **Save a copy** keeps both, when the original still
matters.

## Sharing

### ImgBB (screenshots)

Get a free API key from [api.imgbb.com](https://api.imgbb.com/) and paste it into
Settings. Turn on *Upload every screenshot automatically* and every screenshot
lands on your clipboard as a link, ready to paste into Discord.

Every link is written down. An uploaded screenshot shows its link in the Library
with a **Copy link** button, so losing the one on your clipboard is not the end
of it — ImgBB has no way to list an anonymous key's uploads, and a link nobody
saved is gone for good.

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

CI runs formatting, clippy and tests on every push. Building the installer is
the expensive half, so it only runs when you ask for it: hit **Run workflow** on
the Actions tab to get an installer to try, or push a tag matching `v*` to build
one and publish it as a GitHub release.

See [docs/architecture.md](docs/architecture.md) for the project layout, how the
replay buffer works, and how to type-check the Windows-only code from any
platform.

## Code signing

Releases are unsigned. Signing is worth doing eventually - reputation
accumulates on a certificate and carries across releases, where an unsigned
build starts from zero every time, and signing also cuts down antivirus false
positives.

It is not as simple as buying a certificate any more, and the options are
genuinely constrained by CI. The reasoning, the costs, and how to switch it on
are in [docs/signing.md](docs/signing.md).

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
