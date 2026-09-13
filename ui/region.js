const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const frame = document.getElementById("frame");
const selection = document.getElementById("selection");
const sizeLabel = document.getElementById("size");

let origin = null;
let current = null;
let submitted = false;
/** "capture" to crop a screenshot, "chat" to record where the chat box is. */
let purpose = "capture";

function cancel() {
  if (submitted) return;
  submitted = true;
  invoke("cancel_region_capture");
}

// Reopened rather than recreated, so the reload is driven by an event.
listen("region:open", () => load());

async function load() {
  // The window is reused between captures, so everything from the last one has
  // to go: a stale selection rectangle, and the guard that stops a second
  // submission would otherwise make the overlay inert on its second use.
  submitted = false;
  origin = null;
  current = null;
  selection.hidden = true;
  sizeLabel.hidden = true;
  document.getElementById("error").hidden = true;

  try {
    const { path, purpose: kind } = await invoke("region_frame");
    purpose = kind ?? "capture";
    document.body.classList.toggle("is-chat", purpose === "chat");
    document.getElementById("hint").hidden = purpose !== "chat";
    // Cache-busted: every capture overwrites the same file, so without this the
    // overlay shows the screen as it was the last time it opened.
    frame.src = `${convertFileSrc(path)}?v=${Date.now()}`;
    // Waited for, not assumed. The window is hidden until this resolves, so
    // that the overlay appears with the frozen screen already painted instead
    // of appearing white and filling in afterwards. decode() is the part that
    // actually finishes the work; onload alone can still leave a frame to
    // paint. On reuse it also matters for a second reason: until the new image
    // is decoded the old one is still on screen, so showing early would flash
    // the previous capture.
    await frame.decode().catch(() => {});
  } catch (error) {
    const box = document.getElementById("error");
    box.textContent = String(error);
    box.hidden = false;
    setTimeout(cancel, 2500);
  }
  // Shown either way: an overlay that stays hidden when something went wrong is
  // a hotkey that does nothing, with the reason invisible behind it.
  invoke("region_ready");
}

/// Where the image actually sits inside the window, and how its displayed
/// pixels map back to real ones.
///
/// `object-fit: contain` letterboxes whenever the overlay window and the
/// captured monitor have different shapes, so a click at window coordinates is
/// not a pixel in the source image until it is put through this.
function geometry() {
  const box = frame.getBoundingClientRect();
  const natural = frame.naturalWidth / frame.naturalHeight;
  const shown = box.width / box.height;

  let width = box.width;
  let height = box.height;
  if (shown > natural) {
    width = box.height * natural;
  } else {
    height = box.width / natural;
  }
  return {
    left: box.left + (box.width - width) / 2,
    top: box.top + (box.height - height) / 2,
    width,
    height,
    scale: frame.naturalWidth / width,
  };
}

function clampToImage(clientX, clientY) {
  const g = geometry();
  return {
    x: Math.min(Math.max(clientX, g.left), g.left + g.width),
    y: Math.min(Math.max(clientY, g.top), g.top + g.height),
  };
}

function rectFrom(a, b) {
  return {
    left: Math.min(a.x, b.x),
    top: Math.min(a.y, b.y),
    width: Math.abs(a.x - b.x),
    height: Math.abs(a.y - b.y),
  };
}

function draw() {
  const rect = rectFrom(origin, current);
  Object.assign(selection.style, {
    left: `${rect.left}px`,
    top: `${rect.top}px`,
    width: `${rect.width}px`,
    height: `${rect.height}px`,
  });
  selection.hidden = false;
  selection.classList.toggle("near-top", rect.top < 30);

  const scale = geometry().scale;
  sizeLabel.textContent = `${Math.round(rect.width * scale)} × ${Math.round(rect.height * scale)}`;
}

window.addEventListener("mousedown", (event) => {
  if (event.button !== 0) return;
  origin = clampToImage(event.clientX, event.clientY);
  current = origin;
  document.body.classList.add("is-selecting");
  draw();
});

window.addEventListener("mousemove", (event) => {
  if (!origin) return;
  current = clampToImage(event.clientX, event.clientY);
  draw();
});

window.addEventListener("mouseup", async (event) => {
  if (!origin || event.button !== 0 || submitted) return;

  const g = geometry();
  const rect = rectFrom(origin, clampToImage(event.clientX, event.clientY));
  origin = null;

  // A stray click is a cancel, not a one-pixel screenshot. Written as a
  // positive test so a frame that failed to load - which makes the geometry
  // NaN, and NaN fails every comparison - cancels rather than submitting nulls.
  if (!(rect.width >= 4) || !(rect.height >= 4)) {
    cancel();
    return;
  }

  submitted = true;
  if (purpose === "chat") {
    // The frame's own size, measured off the image that was just dragged over,
    // so the backend can store the rectangle as fractions of it.
    await invoke("finish_chat_region", {
      x: Math.round((rect.left - g.left) * g.scale),
      y: Math.round((rect.top - g.top) * g.scale),
      width: Math.round(rect.width * g.scale),
      height: Math.round(rect.height * g.scale),
      frameWidth: frame.naturalWidth,
      frameHeight: frame.naturalHeight,
    }).catch(() => invoke("cancel_region_capture"));
    return;
  }
  await invoke("finish_region_capture", {
    x: Math.round((rect.left - g.left) * g.scale),
    y: Math.round((rect.top - g.top) * g.scale),
    width: Math.round(rect.width * g.scale),
    height: Math.round(rect.height * g.scale),
  }).catch(() => invoke("cancel_region_capture"));
});

window.addEventListener("keydown", (event) => {
  if (event.key === "Escape") cancel();
});

// Losing focus means something else took over the screen; the frozen frame is
// stale by then, so get out of the way rather than capturing the wrong moment.
window.addEventListener("blur", cancel);
window.addEventListener("contextmenu", (event) => event.preventDefault());

load();
