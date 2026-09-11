# Roadmap

What is queued and what is being considered. Nothing here is a promise; it is
the order things would happen in if nothing changes.

## Next (0.1.x)

Small, and all of it decided.

### Clip length, separate from buffer length

**A defect, not a feature.** `F9` and the Save clip button both pass
`buffer_seconds`, so every clip is the entire buffer. The buffer maximum went to
20 minutes on community request, which at the default 30 Mbps means every press
writes a ~4.5 GB file. Setting a long buffer to be safe currently punishes you
on every clip and then fills the disk cap.

`save_clip` already takes `Option<u32>`; only the caller is wrong. Add
`clip_seconds` (default 60), clamped to the buffer length, and let the buffer be
as long as someone likes.

### Show in folder

`reveal_item` is a registered command with no button anywhere. Library cards
should have one.

### All five hotkeys on the Record tab

Save clip, Screenshot and Region show their key. Session and Toggle buffer do
not, which is part of why rebinding reads as missing.

### Rename a clip in the Library

Filenames are timestamps. For a community that shares clips, "PD chase" beats
`Clip_2026-09-11_00-08-35.mp4`. The only one here with real surface area: the
link index is keyed by file name, and the asset scope needs updating on rename.

## Considered for 2.0

The theme is the difference between a personal recorder and something a whole
community runs.

### A local trigger API

**The big one.** A localhost-only HTTP endpoint, off by default and behind a
token, that accepts "save a clip now" and "start/stop a session", with an
optional label.

That lets the things a community already runs drive the recorder: a Stream Deck
button, a local helper, or - the interesting case - a FiveM server script
telling the client to clip what just happened. Nothing else can do that for
them, because it needs both ends and the community owns both ends.

Hard part is not the endpoint, it is the safety story. Localhost only, a token
generated per install, no remote origin allowed, and a visible indicator when
something else has triggered a recording.

### Markers while recording

A hotkey that drops a timestamp into the current session. The Library then shows
them as chapters, the trim timeline shows them as ticks, and "split at markers"
turns one long session into the four moments worth keeping.

Sessions run for hours. Right now finding the chase at 1h42 means scrubbing for
it, which means most sessions are recorded and never watched. This is the change
that makes session recording pay for itself.

### Separate audio tracks

Game, microphone and voice chat as distinct tracks rather than one mix, so a
clip can be uploaded with the shouting or the copyrighted radio muted, without
re-recording it.

Windows can do per-process loopback capture (`AUDIOCLIENT_ACTIVATION_PARAMS`
with process loopback, Windows 11 and late Windows 10). Meaty but well trodden -
OBS does exactly this.

### Capture the game window, not the screen

Desktop Duplication takes the whole output, second monitor included. Anyone with
Discord open on the side is one clip away from sharing a DM. The Windows
Graphics Capture API captures a single window, needs no hooking, and is what OBS
uses now.

Privacy-aligned with the redaction tools, and removes the "which monitor" setup
question entirely.

### Recovering an interrupted session

A crash or a power cut currently loses the session: the footage is still in the
ring as segments, and the next start clears it. Finding those on launch and
offering to assemble them would turn the worst moment the app has into a good
one.

### Not yet

- **Microphone device selection.** Sounds small, is not: device enumeration, a
  settings UI, and handling a device disappearing mid-recording. Its own
  release.
- **Anything needing a server.** No accounts, no hosting, nothing phoning home -
  that constraint is why this is safe to hand to a community, and it is not
  worth trading for a feature.
- **Posting clips straight to Discord.** A webhook is free and needs no server,
  but Discord caps uploads well below the size of a clip, so it would work for
  screenshots and fail for the thing people actually want to share. Revisit if
  there is a link-based flow worth building.
