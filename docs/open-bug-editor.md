# Open bug: the editor window opens blank and the app becomes unkillable

Unresolved as of the last session. Four attempted fixes, none worked. This is a
record of what has been ruled out, so the next attempt does not repeat them.

## Symptoms

1. Clicking **Hide things** on a screenshot opens a window with the correct
   title bar and a **completely white** client area. No app UI at all - not the
   dark background, not the toolbar.
2. The app then cannot be closed. Not by the window's X, not by the tray's
   Quit. Only killing `FiveMClip.exe` from Task Manager ends it.

Both reproduce every time, on Windows, on a build confirmed to contain the
fixes below.

## Ruled out

**It is not the query string.** The first version passed the image path as
`editor.html?path=...` to `WebviewUrl::App`, which takes a path - so the asset
never resolved. Fixed: the window asks for its target via the `editor_target`
command once loaded. Symptom unchanged.

**It is not the close-to-tray handler.** That was applied to every window
rather than just `main`, so the editor hid itself instead of closing. Fixed and
scoped by label. Symptom unchanged.

**It is not ffmpeg blocking the UI thread.** `save_clip`, the session commands
and region capture all shelled out to ffmpeg from synchronous commands, which
Tauri runs on the main thread. All are async now, and Quit cleans up on its own
thread against a deadline. Symptom unchanged.

**It is not the window-creation logic.** The app was built for Linux and run
under Xvfb. Creating the editor window succeeds from both a background thread
and the main thread, `editor.html` resolves, its JavaScript runs (confirmed by
`editor_target` being invoked), and the event loop does not deadlock. So the
logic is sound on WebKitGTK, and whatever this is, it is specific to WebView2
or to that machine.

**It is not a stale install.** Confirmed against a build whose commit matches.
The build label is now compiled into the binary and shown in Settings, so this
can be checked from a screenshot rather than assumed.

## What has not been tried

- Reading `fivemclip.log` from an affected run. Logging was added in the last
  commit and has not yet produced output from a failing session.
- Right-click, Inspect on the blank window. Devtools are enabled in release
  builds now. If the window responds, the Console and Network tabs answer this
  immediately. If it does not respond, that is itself the answer - the UI
  thread is blocked.
- Task Manager, Details, right-click `FiveMClip.exe`, **Analyze wait chain**.
  Names what the main thread is blocked on.
- Whether the **region overlay** (F11) has ever worked on this machine. Both
  windows are created at runtime, unlike `main` which comes from
  `tauri.conf.json`. If F11 also fails, the fault is "runtime-created windows"
  rather than "the editor", which is a much narrower search.

That last one is the cheapest and most informative. Start there.

## Relevant code

- `src-tauri/src/commands.rs` - `open_editor`, `editor_target`,
  `begin_region_capture`
- `src-tauri/src/main.rs` - `on_window_event`, the tray `quit` handler
- `src-tauri/capabilities/default.json` - lists `main`, `region`, `editor`
- `ui/editor.js` - asks for `editor_target` on load
