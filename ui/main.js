const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const dialog = window.__TAURI__.dialog;

let settings = null;
let libraryItems = [];
let libraryFilter = "all";

/* ---------------- helpers ---------------- */

const $ = (id) => document.getElementById(id);

function formatBytes(bytes) {
  if (!bytes) return "0 MB";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** i;
  return `${value >= 100 || i < 2 ? Math.round(value) : value.toFixed(1)} ${units[i]}`;
}

function formatDuration(seconds) {
  if (seconds < 60) return `${seconds}s`;
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return s ? `${m}m ${s}s` : `${m}m`;
}

let toastTimer;
function toast(message, isError = false) {
  const el = $("toast");
  el.textContent = message;
  el.classList.toggle("error", isError);
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (el.hidden = true), isError ? 6000 : 3000);
}

async function call(command, args) {
  try {
    return await invoke(command, args);
  } catch (error) {
    toast(String(error), true);
    throw error;
  }
}

/* ---------------- tabs ---------------- */

document.querySelectorAll(".tab").forEach((tab) => {
  tab.addEventListener("click", () => {
    document.querySelectorAll(".tab").forEach((t) => t.classList.remove("is-active"));
    document.querySelectorAll(".view").forEach((v) => v.classList.remove("is-active"));
    tab.classList.add("is-active");
    $(`view-${tab.dataset.view}`).classList.add("is-active");
    if (tab.dataset.view === "library") refreshLibrary();
  });
});

/* ---------------- status polling ---------------- */

function renderStatus(status) {
  const dot = $("status-dot");
  const text = $("status-text");
  $("portable-badge").hidden = !status.portable;
  $("app-version").textContent = status.version ?? "—";

  if (!status.ffmpeg_found) {
    dot.className = "dot warn";
    text.textContent = "ffmpeg missing";
  } else if (status.paused_for_disk) {
    dot.className = "dot warn";
    text.textContent = "Paused — low disk";
  } else if (status.running) {
    dot.className = "dot live";
    text.textContent = "Recording";
  } else if (!status.fivem_running && settings?.only_while_fivem_running) {
    dot.className = "dot idle";
    const watched = settings.trigger_processes ?? [];
    // Name the app when there is one worth naming; a list of six is noise.
    text.textContent =
      watched.length === 1
        ? `Waiting for ${watched[0].replace(/\.exe$/i, "")}`
        : "Waiting for a game";
  } else {
    dot.className = "dot idle";
    text.textContent = "Buffer off";
  }

  const target = settings?.buffer_seconds ?? 120;
  const held = Math.min(status.seconds_buffered, target);
  $("buffer-fill").style.width = `${target ? (held / target) * 100 : 0}%`;
  $("buffer-held").textContent = formatDuration(held);
  $("buffer-target").textContent = `of ${formatDuration(target)} buffered`;

  $("buffer-summary").textContent = status.running
    ? `Holding the last ${formatDuration(target)} · about ${formatBytes(status.estimated_buffer_bytes)} on disk`
    : "Not recording.";
  $("btn-toggle-label").textContent = status.running ? "Stop buffer" : "Start buffer";
  $("pipeline-name").textContent = status.pipeline + (status.has_audio ? "" : " · no audio");
  $("disk-summary").textContent = `${formatBytes(status.library_bytes)} of clips and screenshots`;
  const freeLabel = $("disk-free");
  if (status.free_bytes === null || status.free_bytes === undefined) {
    freeLabel.textContent = "";
  } else {
    freeLabel.textContent = `${formatBytes(status.free_bytes)} free on this drive`;
    freeLabel.classList.toggle("tight", status.space !== "fine");
  }

  $("btn-clip").disabled = !status.running;

  const sessionButton = $("btn-session");
  $("btn-session-label").textContent = status.session_active ? "Stop and save" : "Start session";
  sessionButton.classList.toggle("btn-primary", status.session_active);
  // Stopping stays available whatever the buffer is doing: if recording
  // halted for low disk or ffmpeg died, the session is still on disk and
  // saving it is the one thing the user needs to be able to do.
  sessionButton.disabled = status.session_active ? false : !status.running;
  $("btn-session-discard").hidden = !status.session_active;
  $("btn-mark").hidden = !status.session_active;
  $("session-summary").textContent = status.session_active
    ? `Recording ${formatDuration(status.session_seconds)} · ${formatBytes(status.session_bytes)}` +
      (status.session_markers ? ` · ${status.session_markers} marked` : "")
    : "Not recording a session.";

  const alerts = $("alerts");
  const messages = [];
  if (!status.ffmpeg_found) {
    messages.push([
      "error",
      "ffmpeg.exe is missing, so nothing can be recorded. Reinstall FiveMClip, or drop ffmpeg.exe into the app's bin folder.",
    ]);
  }
  if (status.paused_for_disk) {
    messages.push([
      "warn",
      "Recording is paused because the drive is low on space. It resumes on its own once there is room, or lower the limit in Settings.",
    ]);
  } else if (status.space === "low") {
    messages.push([
      "warn",
      "Running low on disk space. Recording will stop before the drive fills up.",
    ]);
  }
  for (const warning of status.warnings ?? []) messages.push(["warn", warning]);

  const signature = JSON.stringify(messages);
  if (alerts.dataset.signature !== signature) {
    alerts.dataset.signature = signature;
    alerts.innerHTML = "";
    for (const [kind, message] of messages) {
      const div = document.createElement("div");
      div.className = `alert alert-${kind}`;
      div.textContent = message;
      alerts.append(div);
    }
  }
}

async function pollStatus() {
  try {
    renderStatus(await invoke("get_status"));
  } catch {
    /* transient during shutdown */
  }
}

/* ---------------- record actions ---------------- */

$("btn-clip").addEventListener("click", async () => {
  const button = $("btn-clip");
  button.disabled = true;
  try {
    await call("save_clip");
    toast("Clip saved");
    refreshLibrary();
  } finally {
    button.disabled = false;
  }
});

$("btn-shot").addEventListener("click", async () => {
  const button = $("btn-shot");
  button.disabled = true;
  try {
    await call("take_screenshot");
    toast(settings.imgbb_auto_upload ? "Screenshot uploaded — link copied" : "Screenshot saved");
    refreshLibrary();
  } finally {
    button.disabled = false;
  }
});

$("btn-region").addEventListener("click", async () => {
  await call("start_region_capture");
  // The overlay covers the screen, so drop this window out of the way first.
  refreshLibrary();
});

$("btn-toggle").addEventListener("click", async () => {
  const status = await invoke("get_status");
  if (status.running) {
    await call("stop_buffer");
  } else {
    await call("start_buffer");
  }
  pollStatus();
});

$("btn-session").addEventListener("click", async () => {
  const button = $("btn-session");
  const status = await invoke("get_status");
  button.disabled = true;
  try {
    if (status.session_active) {
      // Stitching hours of segments takes a moment; say so rather than
      // looking frozen.
      $("btn-session-label").textContent = "Saving…";
      await call("stop_session");
      toast("Session saved");
      refreshLibrary();
    } else {
      await call("start_session");
      toast("Recording a session — press again to stop and save");
    }
  } finally {
    button.disabled = false;
    pollStatus();
  }
});

$("btn-mark").addEventListener("click", async () => {
  const at = await call("mark_session");
  toast(`Marked at ${formatDuration(Math.round(at))}`);
  pollStatus();
});

$("btn-session-discard").addEventListener("click", async () => {
  // Deliberately blunt wording: this throws away everything since the session
  // started, which may be hours.
  const ok = await dialog.confirm(
    "Throw away this session recording? Everything since you started it is lost.",
    { title: "Discard session", kind: "warning" },
  );
  if (!ok) return;
  await call("discard_session");
  toast("Session discarded");
  pollStatus();
});

$("open-log").addEventListener("click", (event) => {
  event.preventDefault();
  call("open_log_folder");
});

$("btn-folder").addEventListener("click", () => call("open_output_folder"));

$("btn-reprobe").addEventListener("click", async () => {
  const button = $("btn-reprobe");
  button.disabled = true;
  button.textContent = "Testing…";
  try {
    const report = await call("reprobe");
    const panel = $("probe-report");
    panel.hidden = false;
    panel.innerHTML = `<strong>${
      report.chosen_label ? `Using ${report.chosen_label}` : "No capture method worked"
    }</strong><ul>${report.attempts
      .map(
        (a) =>
          `<li class="${a.ok ? "ok" : ""}">${a.label}: ${a.ok ? "works" : escapeHtml(a.error ?? "failed")}</li>`,
      )
      .join("")}</ul>`;
  } finally {
    button.disabled = false;
    button.textContent = "Re-test this PC";
  }
});

function escapeHtml(value) {
  const div = document.createElement("div");
  div.textContent = value;
  return div.innerHTML;
}

/* ---------------- library ---------------- */

document.querySelectorAll(".chip").forEach((chip) => {
  chip.addEventListener("click", () => {
    document.querySelectorAll(".chip").forEach((c) => c.classList.remove("is-active"));
    chip.classList.add("is-active");
    libraryFilter = chip.dataset.filter;
    renderLibrary();
  });
});

$("btn-refresh").addEventListener("click", refreshLibrary);

// The editor and the trimmer write from their own windows, so the grid is told
// rather than left showing a thumbnail of a file that no longer looks like it.
listen("library:changed", () => refreshLibrary());

async function refreshLibrary() {
  libraryItems = await invoke("library_items").catch(() => []);
  renderLibrary();
}

function renderLibrary() {
  const grid = $("library-grid");
  const items = libraryItems.filter((i) => libraryFilter === "all" || i.kind === libraryFilter);

  grid.innerHTML = "";
  $("library-empty").hidden = items.length > 0;

  for (const item of items) {
    const card = document.createElement("div");
    card.className = "item";

    const src = convertFileSrc(item.path);
    const isVideo = item.kind === "clip" || item.kind === "session";
    const thumb = isVideo
      ? `<video class="thumb" src="${src}" preload="metadata" muted playsinline></video>`
      : `<img class="thumb" src="${src}" alt="" loading="lazy" />`;

    // Clips and sessions are both MP4 of the same gameplay, so the thumbnails
    // of two recordings taken a minute apart are indistinguishable. Without
    // this the grid reads as duplicates of one file.
    const KINDS = { clip: "Clip", session: "Session", screenshot: "Screenshot" };
    const kind = `<span class="kind kind-${item.kind}">${KINDS[item.kind] ?? item.kind}</span>`;

    const when = new Date(item.modified_ms).toLocaleString();
    const marked = item.marker_count
      ? ` · ${item.marker_count} marked`
      : "";
    // An already-uploaded screenshot keeps its link instead of offering the
    // upload again - a second upload would just orphan the first one on ImgBB.
    // Offered on anything small enough to go as-is. A clip too big for the
    // limit is sent from the trimmer instead, which can shrink it to fit.
    const discord =
      settings?.discord_webhook && item.size_bytes <= (settings.discord_limit_mb ?? 0) * 1e6
        ? `<button class="btn" data-act="discord">To Discord</button>`
        : "";

    const share = isVideo
      ? `<button class="btn" data-act="trim">Trim</button>
         ${discord}
         <button class="btn" data-act="youtube">To YouTube</button>`
      : `<button class="btn" data-act="edit">Edit</button>
         ${
           item.link
             ? `<button class="btn" data-act="copy-link">Copy link</button>`
             : `<button class="btn" data-act="imgbb">Upload</button>`
         }
         ${discord}`;

    const link = item.link
      ? `<span class="link" title="${escapeHtml(item.link.url)}">${escapeHtml(item.link.url)}</span>`
      : "";

    card.innerHTML = `
      <div class="shot">${thumb}${kind}</div>
      <div class="meta">
        <span class="name">${escapeHtml(item.name)}</span>
        <span class="sub">${when} · ${formatBytes(item.size_bytes)}${marked}</span>
        ${link}
      </div>
      <div class="actions">
        <button class="btn" data-act="open">Open</button>
        <button class="btn" data-act="reveal" title="Show this file in Explorer">Folder</button>
        ${share}
        <button class="btn btn-danger" data-act="delete">Delete</button>
      </div>`;

    // Clips double as their own preview: click to play in place.
    const video = card.querySelector("video");
    if (video) {
      video.addEventListener("click", () => {
        video.controls = true;
        video.play();
      });
    }

    card.querySelectorAll("[data-act]").forEach((button) => {
      button.addEventListener("click", () => handleItemAction(button.dataset.act, item, button));
    });
    grid.append(card);
  }
}

async function handleItemAction(action, item, button) {
  switch (action) {
    case "open":
      await call("open_item", { path: item.path });
      break;
    case "reveal":
      await call("reveal_item", { path: item.path });
      break;
    case "edit":
      await call("open_editor", { path: item.path });
      break;
    case "trim":
      await call("open_trimmer", { path: item.path });
      break;
    case "youtube":
      await call("youtube_handoff", { path: item.path });
      toast("YouTube opened — the file path is on your clipboard, paste it into the picker");
      break;
    case "imgbb": {
      button.disabled = true;
      button.textContent = "Uploading…";
      try {
        const result = await call("upload_imgbb", { path: item.path });
        toast(`Uploaded — ${result.url} copied to clipboard`);
        // Redraw so the card shows the link it now has.
        refreshLibrary();
      } finally {
        button.disabled = false;
        button.textContent = "Upload";
      }
      break;
    }
    case "discord": {
      button.disabled = true;
      button.textContent = "Sending…";
      try {
        await call("send_to_discord", { path: item.path, message: "" });
        toast("Sent to Discord");
      } finally {
        button.disabled = false;
        button.textContent = "To Discord";
      }
      break;
    }
    case "copy-link":
      await call("copy_text", { text: item.link.url });
      toast("Link copied to clipboard");
      break;
    case "delete":
      await call("delete_item", { path: item.path });
      refreshLibrary();
      break;
  }
}

/* ---------------- settings ---------------- */

// Output element named explicitly rather than derived from the input's id.
// Deriving it guessed wrong for the two disk sliders - they had no readout at
// all, so "stop recording when free space drops below" did not say below what.
// A silent null is a bad trade for saving an argument.
function bindRange(id, outputId, format) {
  const input = $(id);
  const out = $(outputId);
  if (!input || !out) {
    console.error(`bindRange: missing ${!input ? id : outputId}`);
    return () => {};
  }
  const update = () => {
    out.textContent = format(Number(input.value));
  };
  input.addEventListener("input", () => {
    update();
    if (id === "buffer_seconds" || id === "bitrate_kbps") updateEstimate();
  });
  return update;
}

const updateBufferLabel = bindRange("buffer_seconds", "buffer_seconds_out", formatDuration);
const updateClipLabel = bindRange("clip_seconds", "clip_seconds_out", formatDuration);

/** A clip cannot be longer than the buffer it comes out of. The backend clamps
 *  it anyway, but a slider that lets you pick an impossible number and then
 *  quietly changes it is worse than one that does not offer it. */
function capClipToBuffer() {
  const buffer = Number($("buffer_seconds").value);
  const clip = $("clip_seconds");
  clip.max = String(buffer);
  if (Number(clip.value) > buffer) clip.value = String(buffer);
  updateClipLabel();
}

$("buffer_seconds").addEventListener("input", capClipToBuffer);
const updateBitrateLabel = bindRange("bitrate_kbps", "bitrate_out", (v) => `${(v / 1000).toFixed(0)} Mbps`);
const updateMicLabel = bindRange("mic_gain_db", "mic_gain_out", (v) => `${v > 0 ? "+" : ""}${v} dB`);
const updateMinFreeLabel = bindRange("min_free_gb", "min_free_out", (v) => `${v} GB`);
const updateLibraryCapLabel = bindRange("max_library_gb", "max_library_out", (v) => `${v} GB`);
const updateSystemLabel = bindRange("system_gain_db", "system_gain_out", (v) => `${v > 0 ? "+" : ""}${v} dB`);

function updateEstimate() {
  const seconds = Number($("buffer_seconds").value);
  const kbps = Number($("bitrate_kbps").value);
  const bytes = ((kbps * 1000 * seconds) / 8) * 1.1;
  $("buffer_estimate").textContent = `Uses about ${formatBytes(bytes)} of disk while running.`;
}

$("auto_prune").addEventListener("change", () => {
  $("prune-cap-field").style.display = $("auto_prune").checked ? "" : "none";
});

$("mic_mode").addEventListener("change", () => {
  $("mic-gain-field").style.display = $("mic_mode").value === "off" ? "none" : "";
});

/* ---------------- hotkeys ---------------- */

// What gets stored is built from `event.code`, not `event.key`.
//
// The Rust side parses these with global-hotkey, which names keys the way
// `code` does - KeyK, Digit4, Numpad5, Space, BracketLeft. `key` gives the
// character produced instead, so Numpad5 arrived as "Clear", Space as " " and
// Shift+1 as "Shift+!", none of which parse. Those bindings were saved happily
// and then failed to register.
const CODE_LABELS = {
  Space: "Space",
  Escape: "Esc",
  Enter: "Enter",
  Tab: "Tab",
  Backspace: "Backspace",
  Delete: "Delete",
  Insert: "Insert",
  Home: "Home",
  End: "End",
  PageUp: "Page Up",
  PageDown: "Page Down",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  CapsLock: "Caps Lock",
  NumLock: "Num Lock",
  ScrollLock: "Scroll Lock",
  PrintScreen: "Print Screen",
  Pause: "Pause",
  Backquote: "`",
  Backslash: "\\",
  BracketLeft: "[",
  BracketRight: "]",
  Comma: ",",
  Period: ".",
  Minus: "-",
  Equal: "=",
  Quote: "'",
  Semicolon: ";",
  Slash: "/",
  NumpadAdd: "Numpad +",
  NumpadSubtract: "Numpad -",
  NumpadMultiply: "Numpad *",
  NumpadDivide: "Numpad /",
  NumpadDecimal: "Numpad .",
  NumpadEnter: "Numpad Enter",
};

/** Codes the Rust parser understands. Anything else is refused at capture time
 *  rather than saved and left to fail silently later. */
function isBindable(code) {
  return (
    /^Key[A-Z]$/.test(code) ||
    /^Digit[0-9]$/.test(code) ||
    /^Numpad[0-9]$/.test(code) ||
    /^F([1-9]|1[0-2])$/.test(code) ||
    code in CODE_LABELS
  );
}

/** How a stored combo is shown. "Ctrl+KeyS" reads as Ctrl+S.
 *
 *  Tolerates a missing value. Every hotkey field has a serde default so this
 *  should not happen - but one absent field throwing here takes the whole
 *  settings page down with it, and a blank box is a far better failure. */
function prettyCombo(combo) {
  if (typeof combo !== "string" || combo === "") return "";
  return combo
    .split("+")
    .map((part) => {
      if (/^Key[A-Z]$/.test(part)) return part.slice(3);
      if (/^Digit[0-9]$/.test(part)) return part.slice(5);
      if (/^Numpad[0-9]$/.test(part)) return `Numpad ${part.slice(6)}`;
      return CODE_LABELS[part] ?? part;
    })
    .join("+");
}

/** The input shows a readable label; the accelerator rides along on the
 *  element, because it is the accelerator that has to be saved. */
function setCombo(input, combo) {
  const value = typeof combo === "string" ? combo : "";
  input.dataset.combo = value;
  input.value = prettyCombo(value);
}

/** Keys Windows swallows on the way down, so the webview only ever sees the
 *  key coming back up.
 *
 *  Print Screen is taken by the OS for the clipboard grab and, on Windows 11,
 *  the Snipping Tool. No keydown ever reaches us, so a keydown-only capture
 *  looked like the box was simply ignoring the key - which is exactly what it
 *  was doing. */
const KEYUP_ONLY = new Set(["PrintScreen"]);

function captureCombo(input, event) {
  event.preventDefault();

  // A bare modifier is someone still reaching for the real key.
  if (["Control", "Shift", "Alt", "Meta"].includes(event.key)) return;

  if (event.key === "Escape" && !event.ctrlKey && !event.shiftKey && !event.altKey) {
    setCombo(input, "");
    input.blur();
    return;
  }

  if (!isBindable(event.code)) {
    toast("That key cannot be used as a hotkey — try a function key or a letter", true);
    return;
  }

  const parts = [];
  if (event.ctrlKey) parts.push("Ctrl");
  if (event.shiftKey) parts.push("Shift");
  if (event.altKey) parts.push("Alt");
  if (event.metaKey) parts.push("Super");
  parts.push(event.code);

  setCombo(input, parts.join("+"));
  input.blur();
}

document.querySelectorAll(".hotkey").forEach((input) => {
  input.addEventListener("focus", () => input.classList.add("capturing"));
  input.addEventListener("blur", () => input.classList.remove("capturing"));

  // Both, deliberately. A key that does arrive on the way down is captured
  // there and the box blurs, so its keyup lands elsewhere and cannot capture
  // twice; one that only arrives on the way up is caught below.
  input.addEventListener("keydown", (event) => captureCombo(input, event));
  input.addEventListener("keyup", (event) => {
    if (KEYUP_ONLY.has(event.code)) captureCombo(input, event);
  });
});

$("btn-browse").addEventListener("click", async () => {
  const chosen = await dialog.open({ directory: true, title: "Where should clips be saved?" });
  if (chosen) $("output_dir").value = chosen;
});

$("btn-preview").addEventListener("click", async () => {
  // Save the monitor choice first, otherwise the test shot comes from whichever
  // monitor was configured last time.
  await persistSettings({ silent: true });
  await call("take_screenshot");
  toast("Test shot saved — check the Library tab");
  refreshLibrary();
});

document.querySelectorAll("[data-external]").forEach((link) => {
  link.addEventListener("click", (event) => {
    event.preventDefault();
    call("open_url", { url: link.dataset.external });
  });
});

function applySettings(next) {
  settings = next;
  $("buffer_seconds").value = next.buffer_seconds;
  $("clip_seconds").value = next.clip_seconds;
  $("fps").value = String(next.fps);
  $("bitrate_kbps").value = next.bitrate_kbps;
  $("capture_cursor").checked = next.capture_cursor;
  $("screenshot_jpeg").checked = next.screenshot_jpeg;
  $("copy_screenshot_to_clipboard").checked = next.copy_screenshot_to_clipboard;
  $("edit_after_region").checked = next.edit_after_region;
  $("mic_mode").value = next.mic_mode;
  $("mic_gain_db").value = next.mic_gain_db;
  $("system_gain_db").value = next.system_gain_db;
  setCombo($("hotkey_save_clip"), next.hotkey_save_clip);
  setCombo($("hotkey_screenshot"), next.hotkey_screenshot);
  setCombo($("hotkey_region"), next.hotkey_region);
  setCombo($("hotkey_session"), next.hotkey_session);
  setCombo($("hotkey_marker"), next.hotkey_marker);
  setCombo($("hotkey_toggle_buffer"), next.hotkey_toggle_buffer);
  $("imgbb_api_key").value = next.imgbb_api_key;
  $("imgbb_auto_upload").checked = next.imgbb_auto_upload;
  $("discord_webhook").value = next.discord_webhook;
  $("discord_limit_mb").value = String(next.discord_limit_mb);
  $("min_free_gb").value = next.min_free_gb;
  $("auto_prune").checked = next.auto_prune;
  $("max_library_gb").value = next.max_library_gb;
  $("prune-cap-field").style.display = next.auto_prune ? "" : "none";
  $("only_while_fivem_running").checked = next.only_while_fivem_running;
  triggers = [...(next.trigger_processes ?? [])];
  renderTriggers();
  $("autostart").checked = next.autostart;
  $("start_minimized").checked = next.start_minimized;
  $("output_dir").value = next.output_dir;

  updateBufferLabel();
  capClipToBuffer();
  updateBitrateLabel();
  updateMicLabel();
  updateSystemLabel();
  updateMinFreeLabel();
  updateLibraryCapLabel();
  updateEstimate();
  $("mic-gain-field").style.display = next.mic_mode === "off" ? "none" : "";

  $("hint-clip").textContent = prettyCombo(next.hotkey_save_clip) || "no hotkey";
  $("hint-shot").textContent = prettyCombo(next.hotkey_screenshot) || "no hotkey";
  $("hint-region").textContent = prettyCombo(next.hotkey_region) || "no hotkey";
  $("hint-session").textContent = prettyCombo(next.hotkey_session) || "no hotkey";
  $("hint-toggle").textContent = prettyCombo(next.hotkey_toggle_buffer) || "no hotkey";
  $("hint-marker").textContent = prettyCombo(next.hotkey_marker) || "no hotkey";
}

/* ---------------- trigger apps ---------------- */

let triggers = [];

function renderTriggers() {
  const list = $("trigger-list");
  list.innerHTML = "";
  for (const name of triggers) {
    const chip = document.createElement("span");
    chip.className = "chip-item";
    chip.innerHTML = `${escapeHtml(name)} <button type="button" aria-label="Remove ${escapeHtml(name)}">&times;</button>`;
    chip.querySelector("button").addEventListener("click", () => {
      triggers = triggers.filter((t) => t !== name);
      renderTriggers();
    });
    list.append(chip);
  }
  $("triggers-field").style.display = $("only_while_fivem_running").checked ? "" : "none";
}

function addTrigger() {
  const input = $("trigger-input");
  const name = input.value.trim();
  // Case-insensitive, since Windows executable names are.
  if (name && !triggers.some((t) => t.toLowerCase() === name.toLowerCase())) {
    triggers.push(name);
    renderTriggers();
  }
  input.value = "";
  input.focus();
}

$("trigger-add").addEventListener("click", addTrigger);
$("trigger-input").addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    // Otherwise Enter submits the settings form instead of adding the app.
    event.preventDefault();
    addTrigger();
  }
});
$("only_while_fivem_running").addEventListener("change", renderTriggers);

$("trigger-input").addEventListener("focus", async () => {
  const list = $("running-processes");
  if (list.childElementCount > 0) return;
  const running = await invoke("running_processes").catch(() => []);
  for (const name of running) list.append(new Option(name));
});

function collectSettings() {
  return {
    ...settings,
    buffer_seconds: Number($("buffer_seconds").value),
    clip_seconds: Number($("clip_seconds").value),
    fps: Number($("fps").value),
    bitrate_kbps: Number($("bitrate_kbps").value),
    monitor_index: Number($("monitor_index").value || 0),
    capture_cursor: $("capture_cursor").checked,
    screenshot_jpeg: $("screenshot_jpeg").checked,
    copy_screenshot_to_clipboard: $("copy_screenshot_to_clipboard").checked,
    edit_after_region: $("edit_after_region").checked,
    mic_mode: $("mic_mode").value,
    mic_gain_db: Number($("mic_gain_db").value),
    system_gain_db: Number($("system_gain_db").value),
    // dataset, not value: the box shows "Ctrl+S" and the backend needs
    // "Ctrl+KeyS".
    hotkey_save_clip: $("hotkey_save_clip").dataset.combo ?? "",
    hotkey_screenshot: $("hotkey_screenshot").dataset.combo ?? "",
    hotkey_region: $("hotkey_region").dataset.combo ?? "",
    hotkey_session: $("hotkey_session").dataset.combo ?? "",
    hotkey_marker: $("hotkey_marker").dataset.combo ?? "",
    hotkey_toggle_buffer: $("hotkey_toggle_buffer").dataset.combo ?? "",
    imgbb_api_key: $("imgbb_api_key").value,
    imgbb_auto_upload: $("imgbb_auto_upload").checked,
    discord_webhook: $("discord_webhook").value.trim(),
    discord_limit_mb: Number($("discord_limit_mb").value),
    min_free_gb: Number($("min_free_gb").value),
    auto_prune: $("auto_prune").checked,
    max_library_gb: Number($("max_library_gb").value),
    only_while_fivem_running: $("only_while_fivem_running").checked,
    trigger_processes: triggers,
    autostart: $("autostart").checked,
    start_minimized: $("start_minimized").checked,
    output_dir: $("output_dir").value,
  };
}

async function persistSettings({ silent = false } = {}) {
  const saved = await call("save_settings", { settings: collectSettings() });
  applySettings(saved);
  if (!silent) {
    const badge = $("settings-saved");
    badge.hidden = false;
    setTimeout(() => (badge.hidden = true), 2000);
  }
}

$("settings-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  await persistSettings();
});

/* ---------------- first run ---------------- */

// The setup screen writes into the same Settings object as everything else; it
// is a friendlier front door to three fields, not a separate configuration.
async function refreshSetupEstimate() {
  const seconds = Number($("setup_buffer").value);
  const needed = ((settings.bitrate_kbps * 1000 * seconds) / 8) * 1.1;
  $("setup_buffer_out").textContent = formatDuration(seconds);
  $("setup_estimate").textContent = `Keeps about ${formatBytes(needed)} on disk while running.`;

  const dir = $("setup_output_dir").value;
  const free = dir ? await invoke("disk_free", { path: dir }).catch(() => null) : null;
  const label = $("setup_disk");
  if (free === null || free === undefined) {
    label.textContent = "";
    label.classList.remove("tight");
    return;
  }
  label.textContent = `${formatBytes(free)} free on this drive.`;
  // Leave real headroom: a drive with only the buffer's worth of space left is
  // a drive about to cause problems for everything else on it.
  label.classList.toggle("tight", free < needed * 3);
}

$("setup_buffer").addEventListener("input", refreshSetupEstimate);

$("setup-browse").addEventListener("click", async () => {
  const chosen = await dialog.open({ directory: true, title: "Where should clips be saved?" });
  if (chosen) {
    $("setup_output_dir").value = chosen;
    refreshSetupEstimate();
  }
});

$("setup-done").addEventListener("click", async () => {
  const button = $("setup-done");
  button.disabled = true;
  try {
    const saved = await call("save_settings", {
      settings: {
        ...settings,
        output_dir: $("setup_output_dir").value,
        buffer_seconds: Number($("setup_buffer").value),
        autostart: $("setup_autostart").checked,
        setup_complete: true,
      },
    });
    applySettings(saved);
    // Someone who just finished setup does not need a changelog for the
    // version they installed thirty seconds ago.
    await invoke("dismiss_whats_new").catch(() => {});
    document.body.classList.remove("is-setup");
    $("setup").hidden = true;
    // The watchdog will pick it up within a few seconds, but starting here
    // means the buffer meter moves immediately instead of looking broken.
    await invoke("start_buffer").catch(() => {});
    toast("Ready — press F9 whenever something worth keeping happens");
  } finally {
    button.disabled = false;
  }
});

function showSetup() {
  $("setup_output_dir").value = settings.output_dir;
  $("setup_buffer").value = settings.buffer_seconds;
  $("setup_autostart").checked = settings.autostart;
  document.body.classList.add("is-setup");
  $("setup").hidden = false;
  refreshSetupEstimate();
}

/* ---------------- updates ---------------- */

// Nothing downloads until the button is pressed: restarting a recorder out from
// under someone mid-session is worse than running an old version for another
// day.
async function checkForUpdate() {
  const update = await invoke("check_for_update").catch(() => null);
  if (!update) return;

  $("update-version").textContent = update.version;
  $("update-notes").textContent = (update.notes ?? "").split("\n")[0];
  $("update-banner").hidden = false;
}

// Rechecked while running, not only at launch. This app is built to sit in the
// tray for weeks, so "we look once on startup" means someone who never reboots
// never hears about anything - and the banner is the only way most people find
// out a fix exists.
const UPDATE_RECHECK_MS = 6 * 60 * 60 * 1000;

$("update-install").addEventListener("click", async () => {
  const button = $("update-install");
  button.disabled = true;
  button.textContent = "Downloading…";
  try {
    // On success the app restarts, so nothing after this runs.
    await call("install_update");
  } catch {
    button.disabled = false;
    button.textContent = "Update and restart";
  }
});

/* ---------------- what's new ---------------- */

// Shown once after an update, then stamped so it never reappears for that
// version. Someone who quits without pressing the button sees it again next
// launch, which is the right way round for a note they may not have read.
async function showWhatsNew() {
  const news = await invoke("whats_new").catch(() => null);
  if (!news) return;

  $("whatsnew-version").textContent = news.version;
  const body = $("whatsnew-body");
  body.innerHTML = "";
  // More than one block means the user skipped a version, so each is headed
  // with its own number rather than merged into one undated list.
  const headed = news.releases.length > 1;
  for (const release of news.releases) {
    if (headed) {
      const heading = document.createElement("h3");
      heading.textContent = release.version;
      body.append(heading);
    }
    const list = document.createElement("ul");
    for (const line of release.lines) {
      const li = document.createElement("li");
      li.textContent = line;
      list.append(li);
    }
    body.append(list);
  }
  $("whatsnew").hidden = false;
}

$("whatsnew-done").addEventListener("click", async () => {
  $("whatsnew").hidden = true;
  await invoke("dismiss_whats_new").catch(() => {});
});

/* ---------------- boot ---------------- */

async function boot() {
  applySettings(await invoke("get_settings"));

  if (!settings.setup_complete) {
    showSetup();
  } else {
    // Only ever behind setup: two overlays at once, one of them modal, is how
    // a first run turns into a support question.
    showWhatsNew();
  }

  const monitors = await invoke("list_monitors").catch(() => []);
  const select = $("monitor_index");
  select.innerHTML = "";
  if (monitors.length === 0) {
    select.append(new Option("Primary monitor", "0"));
  } else {
    for (const monitor of monitors) {
      const label = `${monitor.index + 1}. ${monitor.name}${monitor.primary ? " (primary)" : ""}`;
      select.append(new Option(label, String(monitor.index)));
    }
  }
  select.value = String(settings.monitor_index);

  await pollStatus();
  await refreshLibrary();
  setInterval(pollStatus, 1000);

  // Last, and never blocking startup: an unreachable update server should not
  // delay someone getting to the record button.
  checkForUpdate();
  setInterval(checkForUpdate, UPDATE_RECHECK_MS);
}

boot();
