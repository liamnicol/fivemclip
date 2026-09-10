const { invoke, convertFileSrc } = window.__TAURI__.core;
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
  $("btn-toggle").textContent = status.running ? "Stop buffer" : "Start buffer";
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
  sessionButton.textContent = status.session_active ? "Stop and save" : "Start session";
  sessionButton.classList.toggle("btn-primary", status.session_active);
  // Stopping stays available whatever the buffer is doing: if recording
  // halted for low disk or ffmpeg died, the session is still on disk and
  // saving it is the one thing the user needs to be able to do.
  sessionButton.disabled = status.session_active ? false : !status.running;
  $("btn-session-discard").hidden = !status.session_active;
  $("session-summary").textContent = status.session_active
    ? `Recording ${formatDuration(status.session_seconds)} · ${formatBytes(status.session_bytes)}`
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
    await call("save_clip", { seconds: settings.buffer_seconds });
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
      button.textContent = "Saving…";
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

    const when = new Date(item.modified_ms).toLocaleString();
    const share = isVideo
      ? `<button class="btn" data-act="youtube">To YouTube</button>`
      : `<button class="btn" data-act="edit">Hide things</button>
         <button class="btn" data-act="imgbb">Upload</button>`;

    card.innerHTML = `
      ${thumb}
      <div class="meta">
        <span class="name">${escapeHtml(item.name)}</span>
        <span class="sub">${when} · ${formatBytes(item.size_bytes)}</span>
      </div>
      <div class="actions">
        <button class="btn" data-act="open">Open</button>
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
    case "edit":
      await call("open_editor", { path: item.path });
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
      } finally {
        button.disabled = false;
        button.textContent = "Upload";
      }
      break;
    }
    case "delete":
      await call("delete_item", { path: item.path });
      refreshLibrary();
      break;
  }
}

/* ---------------- settings ---------------- */

function bindRange(id, format) {
  const input = $(id);
  const out = $(`${id.replace(/_kbps$|_db$/, "")}_out`) ?? $(`${id}_out`);
  const update = () => {
    if (out) out.textContent = format(Number(input.value));
  };
  input.addEventListener("input", () => {
    update();
    if (id === "buffer_seconds" || id === "bitrate_kbps") updateEstimate();
  });
  return update;
}

const updateBufferLabel = bindRange("buffer_seconds", formatDuration);
const updateBitrateLabel = bindRange("bitrate_kbps", (v) => `${(v / 1000).toFixed(0)} Mbps`);
const updateMicLabel = bindRange("mic_gain_db", (v) => `${v > 0 ? "+" : ""}${v} dB`);
const updateMinFreeLabel = bindRange("min_free_gb", (v) => `${v} GB`);
const updateLibraryCapLabel = bindRange("max_library_gb", (v) => `${v} GB`);
const updateSystemLabel = bindRange("system_gain_db", (v) => `${v > 0 ? "+" : ""}${v} dB`);

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

document.querySelectorAll(".hotkey").forEach((input) => {
  input.addEventListener("focus", () => input.classList.add("capturing"));
  input.addEventListener("blur", () => input.classList.remove("capturing"));
  input.addEventListener("keydown", (event) => {
    event.preventDefault();
    const parts = [];
    if (event.ctrlKey) parts.push("Ctrl");
    if (event.shiftKey) parts.push("Shift");
    if (event.altKey) parts.push("Alt");
    if (event.metaKey) parts.push("Super");

    const key = event.key;
    // A bare modifier is someone still reaching for the real key.
    if (["Control", "Shift", "Alt", "Meta"].includes(key)) return;

    if (key === "Escape") {
      input.value = "";
      input.blur();
      return;
    }
    parts.push(key.length === 1 ? key.toUpperCase() : key);
    input.value = parts.join("+");
    input.blur();
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
  $("fps").value = String(next.fps);
  $("bitrate_kbps").value = next.bitrate_kbps;
  $("capture_cursor").checked = next.capture_cursor;
  $("screenshot_jpeg").checked = next.screenshot_jpeg;
  $("copy_screenshot_to_clipboard").checked = next.copy_screenshot_to_clipboard;
  $("edit_after_region").checked = next.edit_after_region;
  $("mic_mode").value = next.mic_mode;
  $("mic_gain_db").value = next.mic_gain_db;
  $("system_gain_db").value = next.system_gain_db;
  $("hotkey_save_clip").value = next.hotkey_save_clip;
  $("hotkey_screenshot").value = next.hotkey_screenshot;
  $("hotkey_region").value = next.hotkey_region;
  $("hotkey_session").value = next.hotkey_session;
  $("hotkey_toggle_buffer").value = next.hotkey_toggle_buffer;
  $("imgbb_api_key").value = next.imgbb_api_key;
  $("imgbb_auto_upload").checked = next.imgbb_auto_upload;
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
  updateBitrateLabel();
  updateMicLabel();
  updateSystemLabel();
  updateMinFreeLabel();
  updateLibraryCapLabel();
  updateEstimate();
  $("mic-gain-field").style.display = next.mic_mode === "off" ? "none" : "";

  $("hint-clip").textContent = next.hotkey_save_clip || "no hotkey";
  $("hint-shot").textContent = next.hotkey_screenshot || "no hotkey";
  $("hint-region").textContent = next.hotkey_region || "no hotkey";
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
    hotkey_save_clip: $("hotkey_save_clip").value,
    hotkey_screenshot: $("hotkey_screenshot").value,
    hotkey_region: $("hotkey_region").value,
    hotkey_session: $("hotkey_session").value,
    hotkey_toggle_buffer: $("hotkey_toggle_buffer").value,
    imgbb_api_key: $("imgbb_api_key").value,
    imgbb_auto_upload: $("imgbb_auto_upload").checked,
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

// Checked once on launch and never nagged about again. Nothing downloads until
// the button is pressed: restarting a recorder out from under someone
// mid-session is worse than running an old version for another day.
async function checkForUpdate() {
  const update = await invoke("check_for_update").catch(() => null);
  if (!update) return;

  $("update-version").textContent = update.version;
  $("update-notes").textContent = (update.notes ?? "").split("\n")[0];
  $("update-banner").hidden = false;
}

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

/* ---------------- boot ---------------- */

async function boot() {
  applySettings(await invoke("get_settings"));

  if (!settings.setup_complete) {
    showSetup();
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
}

boot();
