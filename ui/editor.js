const { invoke, convertFileSrc } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;
const { listen } = window.__TAURI__.event;

const canvas = document.getElementById("canvas");
const ctx = canvas.getContext("2d");

/** Everything is composed here at full image size and the crop window is then
 *  blitted to the visible canvas. Pixelate and blur read pixels back out of the
 *  canvas they are painting on, and doing that while the canvas is offset by a
 *  crop reads the wrong rectangle - a bug that stays invisible until the first
 *  person crops and redacts in the same session. */
const work = document.createElement("canvas");
const wctx = work.getContext("2d", { willReadFrequently: true });

let sourcePath = "";
let image = null;
/** Committed marks, in order - redactions, annotations and crops alike. Kept as
 *  data rather than baked into the canvas so undo is possible and so each one
 *  is re-rendered from the pristine image every time; stacking effects on top
 *  of each other drifts. */
let marks = [];
let tool = "black";
let dragging = null;

/** Tools that can be reversed, and so warrant the warning banner. */
const REVERSIBLE = new Set(["pixelate", "blur"]);

const MARK_COLOUR = "#ff3b30";
/** Annotations land on gameplay, which is as often a snow-white minimap as a
 *  night street. A dark outline keeps one colour legible on both. */
const MARK_OUTLINE = "rgba(0, 0, 0, 0.55)";

/* ---------------- geometry ---------------- */

// A mark is the two points the user dragged between. An arrow needs to know
// which end is which; everything else only needs the rectangle they span.
function bounds(m) {
  return {
    x: Math.round(Math.min(m.ax, m.bx)),
    y: Math.round(Math.min(m.ay, m.by)),
    w: Math.round(Math.abs(m.ax - m.bx)),
    h: Math.round(Math.abs(m.ay - m.by)),
  };
}

/** The crop in force. Crops are stored in image coordinates and a later crop is
 *  always drawn inside an earlier one, so the newest is the whole answer. */
function activeCrop() {
  for (let i = marks.length - 1; i >= 0; i--) {
    if (marks[i].tool === "crop") return bounds(marks[i]);
  }
  return { x: 0, y: 0, w: work.width, h: work.height };
}

/** Line weight scaled to the image, so a mark on a 4K screenshot is not a
 *  hairline and a mark on a small one does not swallow it. Measured against the
 *  full image rather than the crop, so cropping does not change the weight of
 *  marks already drawn. */
function strokeWidth() {
  return Math.max(4, Math.round(Math.min(work.width, work.height) / 150));
}

/* ---------------- rendering ---------------- */

function render(preview) {
  if (!image) return;

  if (work.width !== image.naturalWidth) work.width = image.naturalWidth;
  if (work.height !== image.naturalHeight) work.height = image.naturalHeight;
  wctx.filter = "none";
  wctx.imageSmoothingEnabled = true;
  wctx.drawImage(image, 0, 0);

  for (const m of marks) apply(m);
  // A crop in progress dims rather than crops: cropping live would move the
  // image out from under the pointer that is still drawing the rectangle.
  if (preview && preview.tool !== "crop") apply(preview);

  const crop = activeCrop();
  if (canvas.width !== crop.w) canvas.width = crop.w;
  if (canvas.height !== crop.h) canvas.height = crop.h;
  ctx.filter = "none";
  ctx.imageSmoothingEnabled = true;
  ctx.drawImage(work, crop.x, crop.y, crop.w, crop.h, 0, 0, crop.w, crop.h);

  if (preview && preview.tool === "crop") previewCrop(bounds(preview), crop);
}

function apply(m) {
  if (m.tool === "crop") return;
  if (m.tool === "arrow") return drawArrow(m);

  const { x, y, w, h } = bounds(m);
  if (w < 1 || h < 1) return;

  if (m.tool === "box") return drawBox(x, y, w, h);

  if (m.tool === "black") {
    wctx.filter = "none";
    wctx.fillStyle = "#000";
    wctx.fillRect(x, y, w, h);
    return;
  }

  if (m.tool === "pixelate") {
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
    sctx.drawImage(work, x, y, w, h, 0, 0, sw, sh);

    wctx.filter = "none";
    wctx.imageSmoothingEnabled = false;
    wctx.drawImage(scratch, 0, 0, sw, sh, x, y, w, h);
    wctx.imageSmoothingEnabled = true;
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
  const sw = Math.min(work.width, x + w + pad) - sx;
  const sh = Math.min(work.height, y + h + pad) - sy;

  const scratch = document.createElement("canvas");
  scratch.width = sw;
  scratch.height = sh;
  scratch.getContext("2d").drawImage(work, sx, sy, sw, sh, 0, 0, sw, sh);

  wctx.save();
  // Clip so the soft edge cannot bleed past the rectangle the user drew.
  wctx.beginPath();
  wctx.rect(x, y, w, h);
  wctx.clip();
  wctx.filter = `blur(${radius}px)`;
  wctx.drawImage(scratch, sx, sy);
  wctx.filter = "none";
  wctx.restore();
}

/* ---------------- annotations ---------------- */

// Every annotation is drawn twice, outline underneath, so it survives whatever
// it lands on. Cheaper than measuring the background and picking a colour.
function drawBox(x, y, w, h) {
  const stroke = strokeWidth();
  wctx.filter = "none";
  wctx.lineJoin = "miter";
  wctx.strokeStyle = MARK_OUTLINE;
  wctx.lineWidth = stroke * 2;
  wctx.strokeRect(x, y, w, h);
  wctx.strokeStyle = MARK_COLOUR;
  wctx.lineWidth = stroke;
  wctx.strokeRect(x, y, w, h);
}

function drawArrow(m) {
  const stroke = strokeWidth();
  const dx = m.bx - m.ax;
  const dy = m.by - m.ay;
  const length = Math.hypot(dx, dy);
  if (length < 1) return;

  const ux = dx / length;
  const uy = dy / length;
  // Sized against the stroke, not the arrow: the shaft carries a dark outline
  // on each side, and a head only a little wider than that reads as a blob
  // rather than a point. Still capped against the arrow's own length, so a
  // short one is a shaft with a head and not one large triangle.
  const head = Math.min(length * 0.6, Math.max(26, stroke * 6));
  const half = head * 0.5;

  // The shaft stops at the back of the head; running it to the tip shows
  // through the point as a bump once the outline is drawn.
  const baseX = m.bx - ux * head;
  const baseY = m.by - uy * head;

  const shaft = new Path2D();
  shaft.moveTo(m.ax, m.ay);
  shaft.lineTo(baseX, baseY);

  const point = new Path2D();
  point.moveTo(m.bx, m.by);
  point.lineTo(baseX - uy * half, baseY + ux * half);
  point.lineTo(baseX + uy * half, baseY - ux * half);
  point.closePath();

  wctx.filter = "none";
  wctx.lineCap = "round";
  wctx.lineJoin = "round";

  wctx.strokeStyle = MARK_OUTLINE;
  wctx.lineWidth = stroke * 2;
  wctx.stroke(shaft);
  wctx.stroke(point);
  wctx.fillStyle = MARK_OUTLINE;
  wctx.fill(point);

  wctx.strokeStyle = MARK_COLOUR;
  wctx.lineWidth = stroke;
  wctx.stroke(shaft);
  wctx.fillStyle = MARK_COLOUR;
  wctx.fill(point);
}

/** Dim everything outside the rectangle being dragged, the way every other
 *  crop tool does, so it is obvious what is about to be thrown away. */
function previewCrop(r, crop) {
  const x = r.x - crop.x;
  const y = r.y - crop.y;
  ctx.save();
  ctx.fillStyle = "rgba(8, 10, 15, 0.62)";
  ctx.beginPath();
  ctx.rect(0, 0, canvas.width, canvas.height);
  ctx.rect(x, y, r.w, r.h);
  ctx.fill("evenodd");
  ctx.strokeStyle = "#fff";
  ctx.lineWidth = Math.max(1, Math.round(strokeWidth() / 2));
  ctx.setLineDash([strokeWidth() * 2, strokeWidth() * 2]);
  ctx.strokeRect(x, y, r.w, r.h);
  ctx.restore();
}

/* ---------------- input ---------------- */

function toImageSpace(event) {
  const box = canvas.getBoundingClientRect();
  const crop = activeCrop();
  const scale = canvas.width / box.width;
  const clamp = (v, max) => Math.min(Math.max(v, 0), max);
  // Offset by the crop: marks are stored against the original image, so that
  // undoing a crop puts everything drawn after it back where it was drawn.
  return {
    x: crop.x + clamp((event.clientX - box.left) * scale, canvas.width),
    y: crop.y + clamp((event.clientY - box.top) * scale, canvas.height),
  };
}

function markBetween(a, b) {
  return { tool, ax: a.x, ay: a.y, bx: b.x, by: b.y };
}

/** Below this a drag is a misclick, not a mark. An arrow is measured along its
 *  length - a vertical one spans no width at all - and a crop needs enough room
 *  left to be worth keeping. */
function tooSmall(m) {
  if (m.tool === "arrow") return Math.hypot(m.bx - m.ax, m.by - m.ay) < 12;
  const { w, h } = bounds(m);
  if (m.tool === "crop") return w < 16 || h < 16;
  return w < 3 || h < 3;
}

canvas.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) return;
  canvas.setPointerCapture(event.pointerId);
  dragging = toImageSpace(event);
});

canvas.addEventListener("pointermove", (event) => {
  if (!dragging) return;
  render(markBetween(dragging, toImageSpace(event)));
});

canvas.addEventListener("pointerup", (event) => {
  if (!dragging) return;
  const mark = markBetween(dragging, toImageSpace(event));
  dragging = null;
  if (tooSmall(mark)) {
    render();
    return;
  }
  marks.push(mark);
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
    document.getElementById("warning").hidden = !REVERSIBLE.has(tool);
  });
});

function refreshButtons() {
  const empty = marks.length === 0;
  document.getElementById("undo").disabled = empty;
  document.getElementById("clear").disabled = empty;
}

function undo() {
  marks.pop();
  refreshButtons();
  render();
}

document.getElementById("undo").addEventListener("click", undo);

document.getElementById("clear").addEventListener("click", () => {
  marks = [];
  refreshButtons();
  render();
});

window.addEventListener("keydown", (event) => {
  if (event.ctrlKey && event.key.toLowerCase() === "z") {
    event.preventDefault();
    undo();
  }
  if (event.key === "Escape") getCurrentWindow().close();
});

/* ---------------- saving ---------------- */

async function save(replace) {
  const buttons = document.querySelectorAll(".bar .btn");
  buttons.forEach((b) => (b.disabled = true));
  try {
    // No preview, so the visible canvas is exactly the cropped, marked image.
    render();
    const dataUrl = canvas.toDataURL("image/png");
    const saved = await invoke("save_edited_image", {
      path: sourcePath,
      pngBase64: dataUrl.slice(dataUrl.indexOf(",") + 1),
      replace,
    });
    sourcePath = saved;
    // Nothing more to undo once it is written; the marks are the file now.
    marks = [];
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
  marks = [];
  refreshButtons();

  const next = new Image();
  next.onload = () => {
    image = next;
    work.width = next.naturalWidth;
    work.height = next.naturalHeight;
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
