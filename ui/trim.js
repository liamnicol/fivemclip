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

  $("reset").disabled = wholeClip();

  // One place decides this. `paint()` and the chat toggle both used to write
  // `fast.disabled` and disagree about it, and paint() runs on every drag - so
  // turning Hide chat on correctly disabled Fast, and the next nudge of a
  // handle quietly turned it back on, leaving the one combination the backend
  // refuses sitting there as an enabled button.
  const why = { save: refusal("save"), fast: refusal("fast") };
  for (const id of ["save", "save-copy"]) {
    $(id).disabled = why.save !== "";
    $(id).title = why.save;
  }
  $("fast").disabled = why.fast !== "";
  $("fast").title = why.fast;

  // A disabled button with no reason given is the whole of "it does not allow
  // saving": Save is grey the moment the window opens, and nothing anywhere
  // says that a selection is what turns it on.
  $("why").textContent = why.save;
  $("why").hidden = why.save === "";
}

const wholeClip = () => start <= 0 && end >= duration;

/** Why an action is refused, ready to show, or "" if it is not. */
function refusal(which) {
  const whole = wholeClip();
  const hiding = hidingChat();

  if (which === "fast") {
    // A stream copy cannot paint over anything; the backend refuses the pair.
    if (hiding) {
      return "A fast trim copies the video without re-encoding, so it cannot black out the chat.";
    }
    if (whole) return "Nothing is trimmed yet — drag the handles to choose what to keep.";
    return "";
  }

  // Saving the whole clip over itself changes nothing - unless it is not only
  // a trim. Hiding the chat re-encodes every frame, so blacking the chat out
  // across a whole clip is a real request and a common one, and refusing it
  // was why the trimmer looked like it would not save at all.
  if (whole && !hiding) {
    return "Nothing is trimmed yet — drag the handles, or scrub and press I and O.";
  }
  return "";
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

/* ---------------- hiding the chat ---------------- */

/** The saved rectangle, as fractions of the frame, or null if none is set. */
let chatRegion = null;

function hidingChat() {
  const box = $("hide-chat");
  return !box.disabled && box.checked;
}

$("hide-chat").addEventListener("change", () => {
  paint();
  paintChatPreview();
});

/** Where the video's picture actually is inside the element.
 *
 *  `object-fit: contain` letterboxes whenever the clip and the box it is drawn
 *  in have different shapes, so a fraction of the frame is not a fraction of
 *  the element until it is put through this. Same problem the region overlay
 *  solves for the frozen screenshot, same shape of answer. */
function pictureBox() {
  const box = video.getBoundingClientRect();
  const natural = video.videoWidth / video.videoHeight;
  if (!Number.isFinite(natural) || natural <= 0) return null;
  const shown = box.width / box.height;
  const width = shown > natural ? box.height * natural : box.width;
  const height = shown > natural ? box.height : box.width / natural;
  return {
    left: box.left + (box.width - width) / 2,
    top: box.top + (box.height - height) / 2,
    width,
    height,
  };
}

/** Show the rectangle that will be painted out, over the video, while the
 *  toggle is on.
 *
 *  There was nothing at all before this: the only way to find out where the
 *  black box would land was to trim the clip and watch the result. A region is
 *  dragged over a frozen desktop in another window, minutes earlier, and
 *  "covering 18% x 22% of the frame" in Settings is not something anyone can
 *  picture. */
function paintChatPreview() {
  const preview = $("chat-preview");
  if (!chatRegion || !hidingChat() || $("timeline").hidden) {
    preview.hidden = true;
    return;
  }
  const picture = pictureBox();
  if (!picture) {
    preview.hidden = true;
    return;
  }
  const stage = $("stage").getBoundingClientRect();
  Object.assign(preview.style, {
    left: `${picture.left - stage.left + chatRegion.x * picture.width}px`,
    top: `${picture.top - stage.top + chatRegion.y * picture.height}px`,
    width: `${chatRegion.w * picture.width}px`,
    height: `${chatRegion.h * picture.height}px`,
  });
  preview.hidden = false;
}

// The picture moves whenever the window does, and a preview that stays where
// the video used to be is worse than none.
window.addEventListener("resize", paintChatPreview);

/** Asked per clip, once it is known: whether it can have its chat hidden
 *  depends on the file, not only on the setting. */
function askAboutChat(path) {
  invoke("chat_hiding", { path })
    .then(({ available, on, region, reason }) => {
      chatRegion = region ?? null;
      // Always shown, never hidden. Hiding it when a region had not been set
      // meant the trimmer offered no chat control and no explanation, which is
      // indistinguishable from the feature being broken.
      $("hide-chat-wrap").hidden = false;
      $("hide-chat-wrap").title = reason ?? "";
      $("hide-chat").disabled = !available;
      $("hide-chat").checked = on;
      $("hide-chat-why").textContent = available ? "" : reason ?? "";
      $("hide-chat-why").hidden = available;
      paint();
      paintChatPreview();
    })
    .catch((error) => {
      // Swallowed before, which left the control hidden and silent.
      $("hide-chat-wrap").hidden = false;
      $("hide-chat").disabled = true;
      $("hide-chat-why").textContent = String(error);
      $("hide-chat-why").hidden = false;
    });
}

/* ---------------- saving ---------------- */

/** "about 2m 10s left", or nothing while there is no basis for saying. */
function remaining(seconds) {
  if (seconds === null || seconds === undefined || !Number.isFinite(seconds)) return "";
  if (seconds < 3) return "almost done";
  if (seconds < 60) return `about ${Math.round(seconds)}s left`;
  const m = Math.floor(seconds / 60);
  const s = Math.round(seconds % 60);
  return `about ${m}m ${String(s).padStart(2, "0")}s left`;
}

function showProgress(fraction, etaSeconds) {
  $("progress").hidden = false;
  const pct = Math.round(Math.max(0, Math.min(1, fraction)) * 100);
  $("progress-fill").style.width = `${pct}%`;
  const left = remaining(etaSeconds);
  $("progress-label").textContent = left ? `Trimming… ${pct}% · ${left}` : `Trimming… ${pct}%`;
}

// The backend reports out of ffmpeg's own -progress output, throttled to eight
// a second. Registered once, at load: a listener added per save would stack up
// one more copy on every attempt.
listen("trim:progress", (event) => {
  const { fraction, eta_seconds } = event.payload ?? {};
  if (typeof fraction === "number") showProgress(fraction, eta_seconds);
});

async function save(replace, fast = false, fitDiscord = false, thenSend = false) {
  const buttons = document.querySelectorAll(".bar .btn");
  const pressed = fitDiscord ? $("discord") : fast ? $("fast") : replace ? $("save") : $("save-copy");
  const label = pressed.textContent;
  buttons.forEach((b) => (b.disabled = true));
  pressed.textContent = "Trimming…";
  // Let go of the file, do not merely stop playing it. Saving writes the result
  // beside the source and renames it into place, and Windows refuses to rename
  // over a file something still has open - which the webview does for as long
  // as the element has a src, paused or not. Pausing was all this did before.
  const at = video.currentTime;
  video.pause();
  video.removeAttribute("src");
  video.load();
  // From zero, and before the first report arrives: a re-encode spends its
  // first seconds opening the file, and a bar that only appears once ffmpeg
  // speaks leaves exactly the silence this is here to remove.
  showProgress(0, null);
  try {
    const channel = chosenChannel();
    const saved = await invoke("trim_clip", {
      path: sourcePath,
      start,
      end,
      replace,
      fast,
      fitBytes: fitDiscord && channel ? channel.limit_bytes : null,
      hideChat: hidingChat(),
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
    $("progress").hidden = true;
    buttons.forEach((b) => (b.disabled = false));
    pressed.textContent = label;
    // Put the clip back. The window stays open after a failure, and without
    // this it stays open showing nothing.
    //
    // With its own metadata handler, not the one `load()` installs: that one
    // sizes a fresh timeline and selects the whole clip, so restoring through
    // it would throw away the selection the user just failed to save.
    video.onloadedmetadata = () => {
      video.currentTime = at;
      paint();
      paintChatPreview();
    };
    video.src = `${convertFileSrc(sourcePath)}?v=${Date.now()}`;
    paint();
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

/** How long the clip is, going and finding out if the header does not say.
 *
 *  A recording whose duration was never written - a session the app was killed
 *  part way through stitching, or one still being flushed - comes back as
 *  `Infinity`. Seeking past the end makes the browser scan for the real one.
 *  This used to give up on the spot and say "could not read how long that clip
 *  is", which turned the trimmer into a dead end for exactly the long sessions
 *  people most want to cut down, with nothing to do about it.
 *
 *  Resolves to null if the seek does not produce an answer, so a file that
 *  genuinely has no end still says so instead of spinning. */
function resolveDuration() {
  const known = () => Number.isFinite(video.duration) && video.duration > 0;
  if (known()) return Promise.resolve(video.duration);

  return new Promise((resolve) => {
    const finish = (value) => {
      clearTimeout(timer);
      video.removeEventListener("durationchange", onChange);
      // Back to the start: the seek that found the duration left the playhead
      // at the end of the clip, which is not where anyone opens a trimmer.
      if (value !== null) {
        try {
          video.currentTime = 0;
        } catch {
          /* a stream that will not seek back is still trimmable */
        }
      }
      resolve(value);
    };
    const onChange = () => known() && finish(video.duration);
    const timer = setTimeout(() => finish(null), 5000);
    video.addEventListener("durationchange", onChange);
    try {
      // Clamped to the end of the media, which is what forces the scan.
      video.currentTime = 1e101;
    } catch {
      finish(null);
    }
  });
}

function load(path) {
  sourcePath = path;
  duration = 0;
  start = 0;
  end = 0;
  marks = [];
  drawMarks();
  askAboutChat(path);
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

  video.onloadedmetadata = async () => {
    const found = await resolveDuration();
    if (found === null) {
      $("loading").textContent = "Could not read how long that clip is.";
      return;
    }
    duration = found;
    end = duration;
    $("loading").hidden = true;
    $("timeline").hidden = false;
    paint();
    paintChatPreview();
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
