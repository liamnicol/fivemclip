// What the front end has to keep doing, checked in a real browser.
//
// Every case here is a bug that shipped. The trimmer in particular looked fine
// in the code and was unusable on screen, which is the sort of thing only a
// layout engine can tell you.
const { open } = require("./open");
const media = require("./media");

const results = [];
const eq = (name, got, want) =>
  results.push({ name, ok: Object.is(got, want), got, want });
const ok = (name, got) => results.push({ name, ok: !!got, got, want: "truthy" });

const CLIP = "C:\\FiveMClip\\Clips\\clip.webm";

function scenario(over = {}) {
  return {
    commands: {
      trim_target: CLIP,
      discord_channels: [],
      markers_for: [],
      chat_hiding: { available: true, on: true, region: { x: 0.03, y: 0.7, w: 0.34, h: 0.26 }, reason: "" },
      trim_clip: "C:\\FiveMClip\\Clips\\clip_trimmed.webm",
      send_to_discord: null,
      ...over,
    },
  };
}

const ready = (p) =>
  p.waitForFunction(() => !document.getElementById("timeline").hidden, null, { timeout: 15000 });

/** Is the middle of `id` actually the thing a click would land on? */
const hits = (p, id) =>
  p.evaluate((id) => {
    const b = document.getElementById(id).getBoundingClientRect();
    const top = document.elementFromPoint(b.x + b.width / 2, b.y + b.height / 2);
    return !!top && (top.id === id || top.closest(`#${id}`) !== null);
  }, id);

async function controlsAreClickable() {
  // The video used to overflow the stage and paint over the transport row, so
  // "Play selection", "Start here" and "End here" did nothing at any window
  // size. Being visible and enabled was never the question.
  for (const size of [{ width: 1080, height: 720 }, { width: 860, height: 580 }]) {
    const h = await open("trim.html", scenario());
    await h.page.setViewportSize(size);
    await ready(h.page);
    for (const id of ["play", "set-start", "set-end", "save", "track"]) {
      eq(`${size.width}x${size.height}: ${id} is clickable`, await hits(h.page, id), true);
    }
    eq(
      `${size.width}x${size.height}: the video stays inside the stage`,
      await h.page.evaluate(() => {
        const v = document.getElementById("video").getBoundingClientRect();
        const s = document.getElementById("stage").getBoundingClientRect();
        return v.bottom <= s.bottom + 1 && v.top >= s.top - 1;
      }),
      true
    );
    await h.done();
  }
}

async function aRecordingWithNoDurationStillOpens() {
  // A session the app was killed part way through writing reports Infinity
  // until something seeks it. The trimmer used to give up and say so.
  media.clipWithoutDuration();
  const h = await open("trim.html", scenario({ trim_target: "C:\\FiveMClip\\Sessions\\no-duration.webm" }));
  await ready(h.page).catch(() => {});
  eq("a durationless recording gets a timeline", await h.page.evaluate(() => !document.getElementById("timeline").hidden), true);
  ok("its duration is a real number", await h.page.evaluate(() => Number.isFinite(window.__duration ?? document.getElementById("video").duration)));
  eq("and it opens at the start, not the end", await h.page.evaluate(() => document.getElementById("video").currentTime < 1), true);
  await h.done();
}

async function theChatBlackoutIsShown() {
  const h = await open("trim.html", scenario());
  await ready(h.page);
  await h.page.waitForTimeout(200);
  eq("the preview is drawn when hiding is on", await h.page.evaluate(() => !document.getElementById("chat-preview").hidden), true);

  // It has to sit over the picture, not over the letterbox and not over the
  // whole stage. Checked against the video's own content box.
  const placed = await h.page.evaluate(() => {
    const v = document.getElementById("video");
    const b = v.getBoundingClientRect();
    const natural = v.videoWidth / v.videoHeight;
    const shown = b.width / b.height;
    const width = shown > natural ? b.height * natural : b.width;
    const height = shown > natural ? b.height : b.width / natural;
    const left = b.left + (b.width - width) / 2;
    const top = b.top + (b.height - height) / 2;
    const p = document.getElementById("chat-preview").getBoundingClientRect();
    const near = (a, c) => Math.abs(a - c) < 2;
    return (
      near(p.left, left + 0.03 * width) &&
      near(p.top, top + 0.7 * height) &&
      near(p.width, 0.34 * width) &&
      near(p.height, 0.26 * height)
    );
  });
  eq("and it lands on the saved fractions of the picture", placed, true);

  await h.page.uncheck("#hide-chat");
  await h.page.waitForTimeout(100);
  eq("it goes when hiding is turned off", await h.page.evaluate(() => document.getElementById("chat-preview").hidden), true);
  await h.done();
}

async function unavailableChatHidingSaysWhy() {
  // Hiding the control meant someone who had ticked the setting and never set
  // a region saw no chat option at all, and no reason for its absence.
  const h = await open("trim.html", scenario({
    chat_hiding: { available: false, on: false, region: null, reason: "No chat region set yet. Settings > Hiding the chat > Set the chat region." },
  }));
  await ready(h.page);
  eq("the control is still there", await h.page.evaluate(() => document.getElementById("hide-chat-wrap").hidden), false);
  eq("it cannot be ticked", await h.page.evaluate(() => document.getElementById("hide-chat").disabled), true);
  ok("and it says why", (await h.page.textContent("#hide-chat-why"))?.includes("No chat region set"));
  await h.done();
}

async function fastAndHidingStayExclusive() {
  // A stream copy cannot paint over anything. The backend refuses the pair;
  // the front end must not offer it.
  const h = await open("trim.html", scenario());
  await ready(h.page);
  eq("Fast is off while hiding is on", await h.page.evaluate(() => document.getElementById("fast").disabled), true);
  await h.page.uncheck("#hide-chat");
  await h.page.waitForTimeout(100);
  eq("and back on when hiding is off", await h.page.evaluate(() => document.getElementById("fast").disabled), false);
  await h.done();
}

async function aFailedTrimLeavesTheWindowUsable() {
  const h = await open("trim.html", scenario({
    trim_clip: () => { throw new Error("ffmpeg said no"); },
    chat_hiding: { available: false, on: false, region: null, reason: "" },
  }));
  await ready(h.page);
  await h.page.evaluate(() => { document.getElementById("video").currentTime = 5; });
  await h.page.waitForTimeout(200);
  await h.page.click("#set-end");
  await h.page.click("#save");
  await h.page.waitForTimeout(400);
  ok("the error is shown", (await h.alerts()).some((a) => a.includes("ffmpeg said no")));
  eq("the window stays open", await h.closed(), 0);
  eq("Save is usable again", await h.page.evaluate(() => document.getElementById("save").disabled), false);
  eq("and says Save again", await h.page.textContent("#save"), "Save");
  // Restoring the clip must not run the handler that opens a fresh timeline,
  // or a refused trim quietly throws the selection away.
  eq("the selection survives", await h.page.textContent("#time-end"), "0:05");
  await h.done();
}

async function theSourceIsReleasedBeforeItIsReplaced() {
  // Windows will not rename over a file another handle has open, and the
  // webview holds one for as long as the element has a src. Pausing is not
  // releasing.
  const h = await open("trim.html", scenario());
  await ready(h.page);
  await h.page.evaluate(() => { document.getElementById("video").currentTime = 5; });
  await h.page.waitForTimeout(200);
  await h.page.click("#set-end");
  await h.page.click("#save");
  await h.page.waitForTimeout(400);
  eq("the video has let go of the file", await h.page.evaluate(() => !document.getElementById("video").getAttribute("src")), true);
  await h.done();
}

/** Enough of a Settings payload for the page to render. */
function settings(over = {}) {
  return {
    buffer_seconds: 300, clip_seconds: 30, fps: 60, bitrate_kbps: 30000,
    monitor_index: 0, capture_cursor: true, screenshot_jpeg: false,
    copy_screenshot_to_clipboard: true, edit_after_region: false,
    mic_mode: "off", mic_gain_db: 0, system_gain_db: 0,
    hotkey_save_clip: "Ctrl+F5", hotkey_screenshot: "Ctrl+F6",
    hotkey_region: "Ctrl+F7", hotkey_session: "Ctrl+F8",
    hotkey_toggle_buffer: "Ctrl+F9", hotkey_marker: "Ctrl+F10",
    imgbb_api_key: "", imgbb_auto_upload: false, discord_targets: [],
    hide_chat: false, chat_region: null, s3: {},
    min_free_gb: 10, auto_prune: false, max_library_gb: 50,
    only_while_fivem_running: false, auto_session: false,
    trigger_processes: [], autostart: false, start_minimized: false,
    output_dir: "C:\\FiveMClip", setup_complete: true,
    ...over,
  };
}

function settingsScenario(over = {}) {
  return {
    commands: {
      get_settings: settings(over),
      get_status: { running: false, session_active: false, message: "" },
      library_items: [], discord_channels: [], list_monitors: [],
      disk_free: { free_gb: 100, total_gb: 500 }, bucket_ready: false,
      running_processes: [], check_for_update: { kind: "UpToDate" },
      whats_new: null, dismiss_whats_new: null, start_buffer: null,
      copy_text: null, start_chat_region_pick: null,
    },
  };
}

async function gameKeyHotkeysAreFlagged() {
  // F8 is the FiveM console. Bound bare, the console opens and then will not
  // close, because a hotkey is not shared with the window that has focus.
  const h = await open("index.html", settingsScenario({ hotkey_session: "F8" }));
  await h.page.waitForTimeout(600);
  eq("a bare F8 binding is called out", await h.page.evaluate(() => document.getElementById("hotkey_session-clash").hidden), false);
  ok("and says what to do", (await h.page.textContent("#hotkey_session-clash"))?.includes("Ctrl"));
  eq("a modified binding is not", await h.page.evaluate(() => document.getElementById("hotkey_save_clip-clash").hidden), true);
  await h.done();
}

async function theDefaultHotkeysAreClean() {
  const h = await open("index.html", settingsScenario());
  await h.page.waitForTimeout(600);
  eq(
    "no shipped default trips the warning",
    await h.page.evaluate(() =>
      [...document.querySelectorAll("small.warn")].every((w) => w.hidden)
    ),
    true
  );
  await h.done();
}

(async () => {
  media.clip();
  for (const test of [
    controlsAreClickable,
    aRecordingWithNoDurationStillOpens,
    theChatBlackoutIsShown,
    unavailableChatHidingSaysWhy,
    fastAndHidingStayExclusive,
    aFailedTrimLeavesTheWindowUsable,
    theSourceIsReleasedBeforeItIsReplaced,
    gameKeyHotkeysAreFlagged,
    theDefaultHotkeysAreClean,
  ]) {
    try {
      await test();
    } catch (e) {
      results.push({ name: test.name, ok: false, got: String(e.message).split("\n")[0], want: "no throw" });
    }
  }

  let failed = 0;
  for (const r of results) {
    if (!r.ok) failed++;
    console.log(`${r.ok ? "ok  " : "FAIL"} ${r.name}${r.ok ? "" : `  (got ${JSON.stringify(r.got)}, want ${JSON.stringify(r.want)})`}`);
  }
  console.log(`\n${results.length - failed}/${results.length} passed`);
  process.exit(failed ? 1 : 0);
})();
