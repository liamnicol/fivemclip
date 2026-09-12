const { invoke, convertFileSrc } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;
const { listen } = window.__TAURI__.event;

const video = document.getElementById("video");
const track = document.getElementById("track");

const $ = (id) => document.getElementById(id);

let sourcePath = "";
let duration = 0;
let start = 0;
let end = 0;
let dragging = null;
/** Offsets, in seconds, marked while the session was recording. */
let marks = [];
/** Set while playing back only the selection, so the end handle acts as a stop
 *  point without that also applying when someone scrubs past it by hand. */
let previewing = false;

/** Below this there is nothing left to watch. Matches trim::MIN_SECONDS. */
const MIN_SECONDS = 0.25;

/** Matches trim::AUDIO_KBPS and trim::MIN_USEFUL_KBPS. Audio is a rounding
 *  error next to video until a size limit forces the bitrate down, at which
 *  point it stops being one. */
const AUDIO_KBPS = 160;
const MIN_USEFUL_KBPS = 1500;

/** Channels to post to. The limit is per channel, so which one is selected
 *  decides the bitrate the clip has to be squeezed to. */
let discordChannels = [];

function chosenChannel() {
  return discordChannels[Number($("discord-target").value)] ?? null;
}

/** The video bitrate that fits the current selection under the limit.
 *  Mirrors trim::bitrate_to_fit, including the 5% it leaves for the muxer. */
function fitKbps(seconds, limitBytes) {
  if (seconds <= 0) return 0;
  return Math.max(0, (limitBytes * 0.95 * 8) / seconds / 1000 - AUDIO_KBPS);
}

/* ---------------- formatting ---------------- */

function clock(seconds) {
  if (!Number.isFinite(seconds)) return "0:00";
  const whole = Math.max(0, Math.floor(seconds));
  const m = Math.floor(whole / 60);
  const s = whole % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

// Tenths, because a trim is often a second or two either way and whole seconds
// hide the difference the handles are being dragged to make.
function precise(seconds) {
  return `${seconds.toFixed(1)}s`;
}

/* ---------------- state ---------------- */

const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), hi);

function paint() {
  const pct = (t) => `${duration ? (t / duration) * 100 : 0}%`;

  $("selection").style.left = pct(start);
  $("selection").style.width = pct(end - start);
  $("outside-left").style.width = pct(start);
  $("outside-right").style.width = pct(duration - end);
  $("handle-start").style.left = pct(start);
  $("handle-end").style.left = pct(end);
  $("playhead").style.left = pct(video.currentTime);
  for (const [i, tick] of [...$("marks").children].entries()) {
    tick.style.left = pct(marks[i]);
  }

  $("time-start").textContent = clock(start);
  $("time-end").textContent = clock(end);
  $("time-now").textContent = clock(video.currentTime);
  $("selected").textContent = precise(end - start);
  $("range").textContent = `of ${precise(duration)} — ${clock(start)} to ${clock(end)}`;

  paintFit();

  const whole = start <= 0 && end >= duration;
  $("reset").disabled = whole;
  // Saving the whole thing over itself is work that changes nothing.
  $("save").disabled = whole;
  $("save-copy").disabled = whole;
  $("fast").disabled = whole;
}

function setStart(t) {
  start = clamp(t, 0, Math.max(0, end - MIN_SECONDS));
  paint();
}

function setEnd(t) {
  end = clamp(t, Math.min(duration, start + MIN_SECONDS), duration);
  paint();
}

/** Say how the selection would land against the Discord limit.
 *
 *  Framed as quality at a computed bitrate rather than "the longest trim that
 *  fits", because at the bitrate this records, what fits is about seven seconds
 *  - a useless thing to tell someone. Keeping the clip and lowering the bitrate
 *  is the answer; this says how much that will cost. */
function paintFit() {
  const pill = $("fit-pill");
  const button = $("discord");
  const picker = $("discord-target");
  const channel = chosenChannel();
  if (!channel) {
    pill.hidden = true;
    button.hidden = true;
    picker.hidden = true;
    return;
  }
  button.hidden = false;
  pill.hidden = false;
  // One channel needs no choosing, and a select with a single option is just
  // furniture.
  picker.hidden = discordChannels.length < 2;

  const kbps = fitKbps(end - start, channel.limit_bytes);
  const mb = (channel.limit_bytes / 1e6).toFixed(0);
  if (kbps < MIN_USEFUL_KBPS) {
    pill.className = "pill bad";
    pill.textContent = `Too long for ${channel.name} at ${mb} MB — try YouTube`;
    button.disabled = true;
    return;
  }
  button.disabled = false;
  const mbps = (kbps / 1000).toFixed(1);
  if (kbps >= 6000) {
    pill.className = "pill good";
    pill.textContent = `Fits ${channel.name} at ${mbps} Mbps`;
  } else {
    pill.className = "pill ok";
    pill.textContent = `Fits ${channel.name} at ${mbps} Mbps — soft`;
  }
}

/** Draw the marks once, on load. They do not move, only the scale does, and
 *  rebuilding them on every paint would throw away the button being hovered. */
function drawMarks() {
  const box = $("marks");
  box.innerHTML = "";
  for (const at of marks) {
    const tick = document.createElement("button");
    tick.className = "mark";
    tick.type = "button";
    tick.title = `Marked at ${clock(at)} — click to jump here`;
    tick.addEventListener("click", (event) => {
      event.stopPropagation();
      previewing = false;
      video.currentTime = at;
      paint();
    });
    box.append(tick);
  }
  const any = marks.length > 0;
  $("prev-mark").hidden = !any;
  $("next-mark").hidden = !any;
}

function jumpMark(forward) {
  const now = video.currentTime;
  // A small margin, so "next" from exactly on a mark does not stay put.
  const next = forward
    ? marks.find((m) => m > now + 0.25)
    : [...marks].reverse().find((m) => m < now - 0.25);
  if (next === undefined) return;
  previewing = false;
  video.currentTime = next;
  paint();
}

$("prev-mark").addEventListener("click", () => jumpMark(false));
$("next-mark").addEventListener("click", () => jumpMark(true));

/* ---------------- the track ---------------- */

function timeAt(event) {
  const box = track.getBoundingClientRect();
  const ratio = clamp((event.clientX - box.left) / box.width, 0, 1);
  return ratio * duration;
}

function beginDrag(which) {
  return (event) => {
    event.preventDefault();
    event.stopPropagation();
    dragging = which;
    event.target.setPointerCapture(event.pointerId);
  };
}

$("handle-start").addEventListener("pointerdown", beginDrag("start"));
$("handle-end").addEventListener("pointerdown", beginDrag("end"));

for (const handle of [$("handle-start"), $("handle-end")]) {
  handle.addEventListener("pointermove", (event) => {
    if (!dragging) return;
    const t = timeAt(event);
    if (dragging === "start") {
      setStart(t);
      // Seek while dragging the start: picking an in-point blind, by reading
      // the clock, is guesswork.
      video.currentTime = start;
    } else {
      setEnd(t);
      video.currentTime = end;
    }
  });
  handle.addEventListener("pointerup", () => (dragging = null));
  handle.addEventListener("pointercancel", () => (dragging = null));

  // Keyboard nudging, for the last tenth of a second that a mouse cannot hit.
  handle.addEventListener("keydown", (event) => {
    const step = event.shiftKey ? 1 : 0.1;
    const which = handle.id === "handle-start" ? setStart : setEnd;
    const at = handle.id === "handle-start" ? () => start : () => end;
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      which(at() - step);
      video.currentTime = at();
    }
    if (event.key === "ArrowRight") {
      event.preventDefault();
      which(at() + step);
      video.currentTime = at();
    }
  });
}

// Clicking the track scrubs, rather than moving whichever handle is nearest.
// Scrubbing is what people do first, and a click that silently redefines the
// selection is a click nobody can undo.
track.addEventListener("pointerdown", (event) => {
  if (dragging) return;
  previewing = false;
  video.currentTime = timeAt(event);
  paint();
});

/* ---------------- transport ---------------- */

$("play").addEventListener("click", () => {
  if (!video.paused) {
    video.pause();
    return;
  }
  previewing = true;
  if (video.currentTime < start || video.currentTime >= end - 0.05) {
    video.currentTime = start;
  }
  video.play();
});

$("set-start").addEventListener("click", () => setStart(video.currentTime));
$("set-end").addEventListener("click", () => setEnd(video.currentTime));

$("reset").addEventListener("click", () => {
  start = 0;
  end = duration;
  paint();
});

video.addEventListener("timeupdate", () => {
  if (previewing && video.currentTime >= end) {
    video.pause();
    video.currentTime = end;
    previewing = false;
  }
  paint();
});

video.addEventListener("play", () => ($("play").textContent = "Pause"));
video.addEventListener("pause", () => ($("play").textContent = "Play selection"));

window.addEventListener("keydown", (event) => {
  // Not while a handle has focus and the arrows are doing something else.
  if (event.key === " ") {
    event.preventDefault();
    $("play").click();
  }
  if (event.key.toLowerCase() === "i") setStart(video.currentTime);
  if (event.key.toLowerCase() === "o") setEnd(video.currentTime);
  if (event.key === "Escape") getCurrentWindow().close();
});

/* ---------------- saving ---------------- */

async function save(replace, fast = false, fitDiscord = false, thenSend = false) {
  const buttons = document.querySelectorAll(".bar .btn");
  const pressed = fitDiscord ? $("discord") : fast ? $("fast") : replace ? $("save") : $("save-copy");
  const label = pressed.textContent;
  buttons.forEach((b) => (b.disabled = true));
  pressed.textContent = "Trimming…";
  video.pause();
  try {
    const channel = chosenChannel();
    const saved = await invoke("trim_clip", {
      path: sourcePath,
      start,
      end,
      replace,
      fast,
      fitBytes: fitDiscord && channel ? channel.limit_bytes : null,
    });
    if (thenSend && channel) {
      pressed.textContent = "Sending…";
      await invoke("send_to_discord", {
        path: saved,
        target: Number($("discord-target").value) || 0,
        message: "",
      });
    }
    getCurrentWindow().close();
  } catch (error) {
    alert(String(error));
    buttons.forEach((b) => (b.disabled = false));
    pressed.textContent = label;
    paint();
  }
}

$("save").addEventListener("click", () => save(true));
$("save-copy").addEventListener("click", () => save(false));
// Replaces, like Save: someone reaching for the fast path on a three hour
// session is not looking to end up with two copies of it.
$("fast").addEventListener("click", () => save(true, true));
// A copy, not a replace: the whole point is a smaller version for Discord, and
// overwriting the good one with a squeezed one is not what anybody meant.
$("discord").addEventListener("click", () => save(false, false, true, true));

/* ---------------- boot ---------------- */

function load(path) {
  sourcePath = path;
  duration = 0;
  start = 0;
  end = 0;
  marks = [];
  drawMarks();
  invoke("markers_for", { path })
    .then((found) => {
      marks = found ?? [];
      drawMarks();
      if (duration) paint();
    })
    .catch(() => {});
  $("timeline").hidden = true;
  $("loading").hidden = false;
  $("loading").textContent = "Loading…";

  video.onloadedmetadata = () => {
    // A clip stitched from segments can report Infinity until it is seeked;
    // without a real duration there is no timeline to draw.
    if (!Number.isFinite(video.duration) || video.duration <= 0) {
      $("loading").textContent = "Could not read how long that clip is.";
      return;
    }
    duration = video.duration;
    end = duration;
    $("loading").hidden = true;
    $("timeline").hidden = false;
    paint();
  };
  video.onerror = () => {
    $("loading").textContent = "Could not open that clip.";
  };
  // Cache-busted: after a save-and-replace the path is unchanged but the file
  // is not, and a stale cached copy would show the untrimmed original.
  video.src = `${convertFileSrc(path)}?v=${Date.now()}`;
}

invoke("discord_channels")
  .then((channels) => {
    discordChannels = channels ?? [];
    const picker = $("discord-target");
    picker.innerHTML = discordChannels
      .map((c, i) => `<option value="${i}">${c.name}</option>`)
      .join("");
    paintFit();
  })
  .catch(() => {});

$("discord-target").addEventListener("change", paintFit);

listen("trim:open", (event) => load(event.payload));

invoke("trim_target")
  .then((path) => {
    if (path) {
      load(path);
    } else {
      $("loading").textContent = "No clip to trim.";
    }
  })
  .catch((error) => {
    $("loading").textContent = String(error);
  });
