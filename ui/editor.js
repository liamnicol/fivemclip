const { invoke, convertFileSrc } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;
const { listen } = window.__TAURI__.event;

const canvas = document.getElementById("canvas");
const ctx = canvas.getContext("2d", { willReadFrequently: true });

let sourcePath = "";
let image = null;
/** Committed redactions, in order. Kept as data rather than baked into the
 *  canvas so undo is possible and so each one is re-rendered from the pristine
 *  image every time - stacking effects on top of each other drifts. */
let redactions = [];
let tool = "black";
let dragging = null;

/* ---------------- rendering ---------------- */

function render(preview) {
  if (!image) return;
  ctx.filter = "none";
  ctx.imageSmoothingEnabled = true;
  ctx.drawImage(image, 0, 0);

  for (const r of redactions) apply(r);
  if (preview) apply(preview);
}

function apply({ tool, x, y, w, h }) {
  if (w < 1 || h < 1) return;

  if (tool === "black") {
    ctx.filter = "none";
    ctx.fillStyle = "#000";
    ctx.fillRect(x, y, w, h);
    return;
  }

  if (tool === "pixelate") {
    // Scale the region down and back up with smoothing off. Block size is
    // driven by the shorter side: a redacted line of chat is only ~20px tall,
    // and blocks bigger than the text are what actually destroys it, while a
    // large area needs coarser blocks to stop shapes reading through.
    const block = Math.min(40, Math.max(8, Math.round(Math.min(w, h) / 6)));
    const sw = Math.max(1, Math.round(w / block));
    const sh = Math.max(1, Math.round(h / block));

    const scratch = document.createElement("canvas");
    scratch.width = sw;
    scratch.height = sh;
    const sctx = scratch.getContext("2d");
    sctx.imageSmoothingEnabled = true;
    sctx.drawImage(canvas, x, y, w, h, 0, 0, sw, sh);

    ctx.filter = "none";
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(scratch, 0, 0, sw, sh, x, y, w, h);
    ctx.imageSmoothingEnabled = true;
    return;
  }

  // Blur. Drawing the canvas onto itself through a filter would smear in
  // neighbouring pixels from outside the selection, so the region is copied
  // out first and blurred on the way back in.
  //
  // Sized against the shorter side, which for a line of chat is roughly the
  // text height. Redaction is not an aesthetic effect: a gentle blur leaves
  // word shapes perfectly readable.
  const radius = Math.max(10, Math.round(Math.min(w, h) / 2));

  // Copy a *larger* region than the selection rather than scaling a smaller
  // one up. Scaling was the first attempt and it magnified the text faster
  // than the blur hid it, so a redacted line came out more legible than an
  // unredacted one. The padding exists only so the blur has real neighbouring
  // pixels at the edges instead of transparency.
  const pad = radius * 2;
  const sx = Math.max(0, x - pad);
  const sy = Math.max(0, y - pad);
  const sw = Math.min(canvas.width, x + w + pad) - sx;
  const sh = Math.min(canvas.height, y + h + pad) - sy;

  const scratch = document.createElement("canvas");
  scratch.width = sw;
  scratch.height = sh;
  scratch.getContext("2d").drawImage(canvas, sx, sy, sw, sh, 0, 0, sw, sh);

  ctx.save();
  // Clip so the soft edge cannot bleed past the rectangle the user drew.
  ctx.beginPath();
  ctx.rect(x, y, w, h);
  ctx.clip();
  ctx.filter = `blur(${radius}px)`;
  ctx.drawImage(scratch, sx, sy);
  ctx.filter = "none";
  ctx.restore();
}

/* ---------------- input ---------------- */

function toImageSpace(event) {
  const box = canvas.getBoundingClientRect();
  const scale = canvas.width / box.width;
  return {
    x: Math.min(Math.max((event.clientX - box.left) * scale, 0), canvas.width),
    y: Math.min(Math.max((event.clientY - box.top) * scale, 0), canvas.height),
  };
}

function rectBetween(a, b) {
  return {
    tool,
    x: Math.round(Math.min(a.x, b.x)),
    y: Math.round(Math.min(a.y, b.y)),
    w: Math.round(Math.abs(a.x - b.x)),
    h: Math.round(Math.abs(a.y - b.y)),
  };
}

canvas.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) return;
  canvas.setPointerCapture(event.pointerId);
  dragging = toImageSpace(event);
});

canvas.addEventListener("pointermove", (event) => {
  if (!dragging) return;
  render(rectBetween(dragging, toImageSpace(event)));
});

canvas.addEventListener("pointerup", (event) => {
  if (!dragging) return;
  const rect = rectBetween(dragging, toImageSpace(event));
  dragging = null;
  // A click is not a redaction.
  if (rect.w < 3 || rect.h < 3) {
    render();
    return;
  }
  redactions.push(rect);
  refreshButtons();
  render();
});

document.querySelectorAll(".tool").forEach((button) => {
  button.addEventListener("click", () => {
    document.querySelectorAll(".tool").forEach((b) => {
      b.classList.remove("is-active");
      b.setAttribute("aria-checked", "false");
    });
    button.classList.add("is-active");
    button.setAttribute("aria-checked", "true");
    tool = button.dataset.tool;
    // Only warn about the tools that warrant it, and only once chosen.
    document.getElementById("warning").hidden = tool === "black";
  });
});

function refreshButtons() {
  const empty = redactions.length === 0;
  document.getElementById("undo").disabled = empty;
  document.getElementById("clear").disabled = empty;
}

document.getElementById("undo").addEventListener("click", () => {
  redactions.pop();
  refreshButtons();
  render();
});

document.getElementById("clear").addEventListener("click", () => {
  redactions = [];
  refreshButtons();
  render();
});

window.addEventListener("keydown", (event) => {
  if (event.ctrlKey && event.key.toLowerCase() === "z") {
    event.preventDefault();
    redactions.pop();
    refreshButtons();
    render();
  }
  if (event.key === "Escape") getCurrentWindow().close();
});

/* ---------------- saving ---------------- */

async function save(replace) {
  const buttons = document.querySelectorAll(".bar .btn");
  buttons.forEach((b) => (b.disabled = true));
  try {
    render();
    const dataUrl = canvas.toDataURL("image/png");
    const saved = await invoke("save_edited_image", {
      path: sourcePath,
      pngBase64: dataUrl.slice(dataUrl.indexOf(",") + 1),
      replace,
    });
    sourcePath = saved;
    // Nothing more to undo once it is written; the redactions are the file now.
    redactions = [];
    getCurrentWindow().close();
  } catch (error) {
    alert(String(error));
    buttons.forEach((b) => (b.disabled = false));
    refreshButtons();
  }
}

document.getElementById("save").addEventListener("click", () => save(true));
document.getElementById("save-copy").addEventListener("click", () => save(false));

/* ---------------- boot ---------------- */

function load(path) {
  sourcePath = path;
  redactions = [];
  refreshButtons();

  const next = new Image();
  next.onload = () => {
    image = next;
    canvas.width = next.naturalWidth;
    canvas.height = next.naturalHeight;
    document.getElementById("loading").hidden = true;
    render();
  };
  next.onerror = () => {
    document.getElementById("loading").textContent = "Could not open that image.";
  };
  // Cache-busted: after a save-and-replace the path is unchanged but the file
  // is not, and a stale cached copy would show the unredacted original.
  next.src = `${convertFileSrc(path)}?v=${Date.now()}`;
}

listen("editor:open", (event) => load(event.payload));

invoke("editor_target")
  .then((path) => {
    if (path) {
      load(path);
    } else {
      document.getElementById("loading").textContent = "No image to edit.";
    }
  })
  .catch((error) => {
    document.getElementById("loading").textContent = String(error);
  });
