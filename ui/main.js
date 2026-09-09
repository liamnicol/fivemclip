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

  if (!status.ffmpeg_found) {
    dot.className = "dot warn";
    text.textContent = "ffmpeg missing";
  } else if (status.running) {
    dot.className = "dot live";
    text.textContent = "Recording";
  } else if (!status.fivem_running && settings?.only_while_fivem_running) {
    dot.className = "dot idle";
    text.textContent = "Waiting for FiveM";
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

  $("btn-clip").disabled = !status.running;

  const alerts = $("alerts");
  const messages = [];
  if (!status.ffmpeg_found) {
    messages.push([
      "error",
      "ffmpeg.exe is missing, so nothing can be recorded. Reinstall FiveMClip, or drop ffmpeg.exe into the app's bin folder.",
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

$("btn-toggle").addEventListener("click", async () => {
  const status = await invoke("get_status");
  if (status.running) {
    await call("stop_buffer");
  } else {
    await call("start_buffer");
  }
  pollStatus();
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
    const thumb =
      item.kind === "clip"
        ? `<video class="thumb" src="${src}" preload="metadata" muted playsinline></video>`
        : `<img class="thumb" src="${src}" alt="" loading="lazy" />`;

    const when = new Date(item.modified_ms).toLocaleString();
    const share =
      item.kind === "clip"
        ? `<button class="btn" data-act="youtube">To YouTube</button>`
        : `<button class="btn" data-act="imgbb">Upload</button>`;

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
const updateSystemLabel = bindRange("system_gain_db", (v) => `${v > 0 ? "+" : ""}${v} dB`);

function updateEstimate() {
  const seconds = Number($("buffer_seconds").value);
  const kbps = Number($("bitrate_kbps").value);
  const bytes = ((kbps * 1000 * seconds) / 8) * 1.1;
  $("buffer_estimate").textContent = `Uses about ${formatBytes(bytes)} of disk while running.`;
}

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
  $("mic_mode").value = next.mic_mode;
  $("mic_gain_db").value = next.mic_gain_db;
  $("system_gain_db").value = next.system_gain_db;
  $("hotkey_save_clip").value = next.hotkey_save_clip;
  $("hotkey_screenshot").value = next.hotkey_screenshot;
  $("hotkey_toggle_buffer").value = next.hotkey_toggle_buffer;
  $("imgbb_api_key").value = next.imgbb_api_key;
  $("imgbb_auto_upload").checked = next.imgbb_auto_upload;
  $("only_while_fivem_running").checked = next.only_while_fivem_running;
  $("autostart").checked = next.autostart;
  $("start_minimized").checked = next.start_minimized;
  $("output_dir").value = next.output_dir;

  updateBufferLabel();
  updateBitrateLabel();
  updateMicLabel();
  updateSystemLabel();
  updateEstimate();
  $("mic-gain-field").style.display = next.mic_mode === "off" ? "none" : "";

  $("hint-clip").textContent = next.hotkey_save_clip || "no hotkey";
  $("hint-shot").textContent = next.hotkey_screenshot || "no hotkey";
}

function collectSettings() {
  return {
    ...settings,
    buffer_seconds: Number($("buffer_seconds").value),
    fps: Number($("fps").value),
    bitrate_kbps: Number($("bitrate_kbps").value),
    monitor_index: Number($("monitor_index").value || 0),
    capture_cursor: $("capture_cursor").checked,
    screenshot_jpeg: $("screenshot_jpeg").checked,
    mic_mode: $("mic_mode").value,
    mic_gain_db: Number($("mic_gain_db").value),
    system_gain_db: Number($("system_gain_db").value),
    hotkey_save_clip: $("hotkey_save_clip").value,
    hotkey_screenshot: $("hotkey_screenshot").value,
    hotkey_toggle_buffer: $("hotkey_toggle_buffer").value,
    imgbb_api_key: $("imgbb_api_key").value,
    imgbb_auto_upload: $("imgbb_auto_upload").checked,
    only_while_fivem_running: $("only_while_fivem_running").checked,
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

/* ---------------- boot ---------------- */

async function boot() {
  applySettings(await invoke("get_settings"));

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
}

boot();
