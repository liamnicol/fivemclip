# Handoff: crashes and corrupt sessions on Windows

> **Update.** The region overlay part of this is **done** - four logs from a
> 0.2.14 run show the freeze before both branches, no watchdog line, and no
> delay. What is left is below; the region history is kept for context.
>
> The live questions are now **corrupt session recordings** and an app that
> reportedly crashes. On the crash: four logs covering a full day contain no
> crash at all. Two runs end in an update install and one exits cleanly. If it
> is crashing, it is doing so in a way that leaves no log, so the first job is
> to catch one - `fivemclip.log.1` through `.5` immediately after it happens,
> before an update rotates them away.
>
> 0.2.15 makes the capture core log for the first time. Before it, nothing in
> `crates/capture` logged anything, so recording, sessions, ffmpeg restarts and
> stitching were all invisible. Get a fresh log from 0.2.15 with a session in it
> before concluding anything.

# Appendix: verify the region overlay on Windows

Written by a Claude Code web session that has no Windows machine, for a session
that does. Everything below was reasoned about and unit-tested on Linux; the
part that matters was never seen running. Delete this file once it is done.

Read `CLAUDE.md` first - it carries the standing rules and is the more important
document. This one is only the current state and what is still unproven.

## Where things stand

`main` is at **0.2.14**. Released tags run to v0.2.12, so 0.2.13 and 0.2.14 are
unreleased at time of writing.

Region capture (Print Screen -> drag a rectangle) broke in 0.2.12 and the fix is
in 0.2.14. The short version of four versions of thrashing:

| | what happened |
|---|---|
| 0.2.11 | Overlay rebuilt per capture. Slow, so it was changed to hide and reuse. |
| 0.2.12 | Reuse landed, but the screen froze only on the build path, so every capture after the first opened onto "The captured frame is missing". Fixed by moving the freeze before both branches - **that part is good and confirmed by the user's log.** |
| 0.2.12 | Same release also built the overlay `.visible(false)` and showed it once the page reported it had painted, to kill a white flash. **This cannot work: a hidden WebView2 window does not run its page at all.** The report never came, and every capture sat on a 1.5s fallback timer before appearing. |
| 0.2.14 | Handshake and fallback timer both deleted. Window is shown immediately again. |

The user's log named it exactly:

```
04:13:31.953  end building the region overlay (1 ms)
04:13:33.455  the region overlay never reported ready; showing it anyway
```

That second line no longer exists anywhere in the source. **If you ever see it,
the running build is older than 0.2.14.**

## What 0.2.14 does about the white flash, and why you are needed

The flash has two separate causes and they are now handled in two places:

1. **White before the page has rendered** is the webview's own background.
   `commands.rs` builds the window with `.background_color(Color(12, 14, 18, 255))`.
2. **The previous capture flashing up on reuse** is a page problem, because the
   window is already visible by then. `ui/region.js` adds `body.is-loading` on
   `region:open` and removes it only once `img.decode()` resolves, so `#frame`
   stays hidden until the new image is ready.

Cause 2 is verified in a browser harness. **Cause 1 is not verified at all.**
`background_color` is the right mechanism on paper and it compiles against
Tauri 2.11.5, but nobody has watched it happen over a running game. That is the
main reason for this handoff.

## What to check

Build and run it (`./tools/fetch-ffmpeg.ps1` first, then `cargo tauri dev`), with
FiveM or any dark fullscreen thing behind it, and press Print Screen.

1. **Does the overlay appear immediately?** It should be effectively instant. A
   pause of about a second and a half is the 0.2.12 bug and means you are not on
   this build.
2. **Is there a white flash as it opens?** This is the unproven one. Watch a few
   times; it is one frame.
3. **Press Print Screen again, several times.** The second and later captures are
   the reuse path. Look for the *previous* capture appearing for a frame before
   the new one, and for "The captured frame is missing".
4. **Does the frozen frame match the screen** at the moment the key was pressed,
   on every capture and not just the first?
5. **Cancel with Escape, then capture again.** The overlay's `submitted` guard
   has to reset or the second use is inert.

The log is `%APPDATA%\com.fivemclip.desktop\fivemclip.log` (beside the exe if
portable), with `.1` to `.5` for previous runs. A healthy capture shows
`freezing the screen for the region overlay` before *either*
`reusing the region overlay` or `building the region overlay` - the freeze span
appearing only next to the build line is the 0.2.12 bug returning.

## If the flash is still there

Do not go back to building the window hidden. It is not that it was done badly;
the page does not run, so there is nothing to wait for. Worth trying instead, in
order:

- Set a dark background in `region.html`'s own `<style>` as well - cheap, and
  rules out the page's first paint being the white one rather than the webview's.
- Build the window **off-screen** and move it into place once the page reports
  ready. An off-screen window is not hidden, so its page does run, which is the
  exact trap the 0.2.12 version fell into. This needs the `region_ready` command
  back; it was deleted in `ed356df` if you want to see what it looked like.
- Set WebView2's `DefaultBackgroundColor` directly through the COM interface, if
  Tauri's `background_color` turns out not to reach it.

Whichever it is, please write what Windows actually proved into `CLAUDE.md`. The
existing note there says a hidden window does not run its page; if the off-screen
variant works, that is worth recording next to it, and if `background_color`
turns out to do nothing, that is worth recording even more.

## Ground rules that cost real time to learn

All of these are in `CLAUDE.md` in full; these are the ones this area trips over.

- **Never build a window from the main thread.** `build()` waits for the event
  loop, so on the event loop thread it deadlocks into an unkillable process.
  `async` does not save you. `begin_region_capture` spawns an explicit thread.
  `docs/open-bug-editor.md` is four failed fixes on this; read it before changing
  window creation.
- **Long-lived windows are hidden and reused, never closed and rebuilt.**
  `close()` is async, so the next open finds no window while the label is taken.
- **Everything a reused overlay needs must be prepared before the reuse branch.**
  That is the 0.2.12 missing-frame bug.
- **Print Screen only arrives as a `keyup`** - Windows takes the keydown.
- **The browser harness in `ui/` cannot see any of this.** There is no hidden
  window state there and the page always runs. It is good for page logic and it
  passed every test while this was broken on Windows.

Before pushing: `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test --workspace`. To ship, bump the
version in **both** `Cargo.toml` and `src-tauri/tauri.conf.json`, add a
`whatsnew::RELEASES` entry (a test fails without one), then tag and push.
