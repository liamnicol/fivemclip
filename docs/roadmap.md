# Roadmap

What is queued and what is being considered. Nothing here is a promise; it is
the order things would happen in if nothing changes.

## Next (0.1.x)

### Rename a clip in the Library

Filenames are timestamps. For a community that shares clips, "PD chase" beats
`Clip_2026-09-11_00-08-35.mp4`. The link index is keyed by file name and the
asset scope needs updating on rename, so it is not quite the one-liner the other
three were.

### Done

- **Clip length, separate from buffer length** (0.1.9). Was a defect: every clip
  was the whole buffer, so a 20 minute buffer meant a ~4.5 GB file per press.
- **Show in folder** (0.1.9). `reveal_item` had been a registered command with
  no button.
- **All five hotkeys on the Record tab** (0.1.9).

## 2.0

Two features, both decided.

### Markers while recording — done in 0.2.0

A hotkey that drops a timestamp into the running session. The Library shows the
count, the trim timeline shows them as ticks you can click to jump between, and
eventually "split at markers" turns one long session into the four moments worth
keeping.

Sessions run for hours. Finding the chase at 1h42 currently means scrubbing for
it, which means most sessions get recorded and never watched. This is the change
that makes session recording pay for itself.

Markers live in a sidecar index beside the settings, keyed by file name, the same
shape as the ImgBB link index - a session file is not a place to put metadata we
want to edit later.

### Sending a clip to Discord

A webhook URL in Settings and a **Send to Discord** button. No server, no
account, no bot: a webhook is a URL that posts to one channel, so the whole
feature is one multipart POST.

The URL is a secret in the same way the ImgBB key is - anyone holding it can post
to that channel - so it is stored locally, shown as a password field, and never
leaves the machine except to Discord.

**The size limit is the whole design.** Discord caps uploads well below the size
of a clip. At the default 30 Mbps:

| Discord limit | Seconds that fit |
| --- | --- |
| 10 MB | 2.7 s |
| 25 MB | 6.6 s |
| 50 MB | 13.3 s |
| 100 MB | 26.5 s |

So showing "the longest trim that fits" is useless - it would tell people they
may keep seven seconds. The fix is the other way round: keep the clip you want
and **re-encode to fit**, since the trimmer already re-encodes by default.
Bitrate needed, in Mbps:

| Length | 10 MB | 25 MB | 50 MB | 100 MB |
| --- | --- | --- | --- | --- |
| 15s | 5.2 | 13.2 | 26.5 | 53.2 |
| 30s | 2.5 | 6.5 | 13.2 | 26.5 |
| 1m | 1.2 | 3.2 | 6.5 | 13.2 |
| 2m | 0.5 | 1.5 | 3.2 | 6.5 |
| 5m | 0.1 | 0.5 | 1.2 | 2.5 |

Thirty seconds into 25 MB is 6.5 Mbps, which is a perfectly good 1080p clip. A
minute is 3.2 Mbps, which is soft but watchable. Five minutes is not worth
sending to Discord at all, and the app should say so and point at the YouTube
hand-off instead of quietly producing a smear.

So the trimmer gains a live readout of the estimated size against the chosen
limit, and a save mode that computes the bitrate rather than using the capture
one. It refuses, with a reason, below roughly 1.5 Mbps.

The limit itself is a setting, not a constant. Discord has changed it more than
once and it varies by Nitro tier and server boost level, so it is a number the
user picks, with presets, defaulting to the most conservative.

## Considered later

### Separate audio tracks

Game, microphone and voice chat as distinct tracks rather than one mix, so a clip
can be uploaded with the shouting or the copyrighted radio muted, without
re-recording it.

Windows can do per-process loopback capture (`AUDIOCLIENT_ACTIVATION_PARAMS` with
process loopback, Windows 11 and late Windows 10). Meaty but well trodden - OBS
does exactly this.

### Capture the game window, not the screen

Desktop Duplication takes the whole output, second monitor included. Anyone with
Discord open on the side is one clip away from sharing a DM. The Windows Graphics
Capture API captures a single window, needs no hooking, and is what OBS uses now.

Privacy-aligned with the redaction tools, and it removes the "which monitor"
setup question entirely.

### Recovering an interrupted session

A crash or a power cut currently loses the session: the footage is still in the
ring as segments, and the next start clears it. Finding those on launch and
offering to assemble them would turn the worst moment the app has into a good
one.

### Not doing

- **A local trigger API.** A localhost endpoint letting a FiveM server script
  clip what just happened is the most interesting thing this app could do, and
  it is not ours to build: it needs the server side, and that is someone else's
  roadmap. A feature that depends on another team's priorities is a dependency,
  not a feature. Revisit only if those devs ask for it.
- **Microphone device selection.** Sounds small, is not: device enumeration, a
  settings UI, and handling a device disappearing mid-recording. Its own
  release.
- **Anything needing a server.** No accounts, no hosting, nothing phoning home -
  that constraint is why this is safe to hand to a community, and it is not
  worth trading for a feature.
