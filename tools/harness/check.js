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

async function theWholeClipCanStillHaveItsChatHidden() {
  // Blacking the chat out across a whole clip re-encodes every frame, so it is
  // a real request - and the commonest one. Save was disabled whenever nothing
  // was trimmed, which refused it and looked like the trimmer not saving.
  const h = await open("trim.html", scenario());
  await ready(h.page);
  await h.page.waitForTimeout(200);
  eq("Save works on an untrimmed clip while hiding the chat", await h.page.evaluate(() => document.getElementById("save").disabled), false);

  await h.page.uncheck("#hide-chat");
  await h.page.waitForTimeout(150);
  eq("and is refused once there is nothing to do", await h.page.evaluate(() => document.getElementById("save").disabled), true);
  ok("with the reason shown", (await h.page.textContent("#why"))?.includes("Nothing is trimmed yet"));
  await h.done();
}

async function fastStaysOffAcrossADrag() {
  // paint() and the chat toggle both wrote fast.disabled; paint() runs on
  // every drag, so Fast came back to life on the first nudge of a handle.
  const h = await open("trim.html", scenario());
  await ready(h.page);
  await h.page.waitForTimeout(200);
  eq("Fast is off while hiding is on", await h.page.evaluate(() => document.getElementById("fast").disabled), true);

  const box = await h.page.locator("#track").boundingBox();
  const he = await h.page.locator("#handle-end").boundingBox();
  await h.page.mouse.move(he.x + he.width / 2, he.y + he.height / 2);
  await h.page.mouse.down();
  await h.page.mouse.move(box.x + box.width * 0.6, he.y + he.height / 2, { steps: 12 });
  await h.page.mouse.up();
  await h.page.waitForTimeout(200);
  eq("and is still off after dragging a handle", await h.page.evaluate(() => document.getElementById("fast").disabled), true);
  await h.done();
}

async function draggingAHandleEnablesSaving() {
  const h = await open("trim.html", scenario({
    chat_hiding: { available: false, on: false, region: null, reason: "" },
  }));
  await ready(h.page);
  await h.page.waitForTimeout(200);
  eq("Save starts refused", await h.page.evaluate(() => document.getElementById("save").disabled), true);

  const box = await h.page.locator("#track").boundingBox();
  const he = await h.page.locator("#handle-end").boundingBox();
  await h.page.mouse.move(he.x + he.width / 2, he.y + he.height / 2);
  await h.page.mouse.down();
  await h.page.mouse.move(box.x + box.width * 0.6, he.y + he.height / 2, { steps: 12 });
  await h.page.mouse.up();
  await h.page.waitForTimeout(200);
  eq("and a drag turns it on", await h.page.evaluate(() => document.getElementById("save").disabled), false);
  eq("the reason goes with it", await h.page.evaluate(() => document.getElementById("why").hidden), true);
  await h.done();
}

async function fastAndHidingStayExclusive() {
  // A stream copy cannot paint over anything. The backend refuses the pair;
  // the front end must not offer it.
  const h = await open("trim.html", scenario());
  await ready(h.page);
  await h.page.waitForTimeout(200);
  eq("Fast is off while hiding is on", await h.page.evaluate(() => document.getElementById("fast").disabled), true);

  // Trimmed as well as unhidden: a fast copy of a whole untrimmed clip is a
  // file copy, so it stays refused for that reason on its own.
  await h.page.uncheck("#hide-chat");
  await h.page.evaluate(() => { document.getElementById("video").currentTime = 12; });
  await h.page.waitForTimeout(200);
  await h.page.click("#set-end");
  await h.page.waitForTimeout(100);
  eq("and back on once hiding is off and something is trimmed", await h.page.evaluate(() => document.getElementById("fast").disabled), false);
  await h.done();
}

async function aTrimShowsHowFarAlongItIs() {
  // "Trimming…" and then nothing for four minutes is indistinguishable from a
  // hang, which is what a long re-encode looked like.
  let release;
  const held = new Promise((r) => (release = r));
  const h = await open("trim.html", scenario({
    // Held open so the in-flight state can be inspected, the way a real
    // re-encode of a long session is held open for minutes.
    trim_clip: () => new Promise(() => {}),
    chat_hiding: { available: false, on: false, region: null, reason: "" },
  }));
  await ready(h.page);
  await h.page.evaluate(() => { document.getElementById("video").currentTime = 12; });
  await h.page.waitForTimeout(200);
  await h.page.click("#set-end");
  await h.page.click("#save");
  await h.page.waitForTimeout(200);

  eq("the bar appears as soon as it starts", await h.page.evaluate(() => document.getElementById("progress").hidden), false);
  eq("starting at zero", await h.page.evaluate(() => document.getElementById("progress-fill").style.width), "0%");

  await h.emit("trim:progress", { fraction: 0.42, eta_seconds: 95 });
  await h.page.waitForTimeout(100);
  eq("it follows what the backend reports", await h.page.evaluate(() => document.getElementById("progress-fill").style.width), "42%");
  const label = await h.page.textContent("#progress-label");
  ok("and says how long is left", label.includes("42%") && label.includes("1m 35s"));

  await h.emit("trim:progress", { fraction: 1.0, eta_seconds: 0 });
  await h.page.waitForTimeout(100);
  ok(
    "at 100% it says it is finishing rather than repeating 100%",
    (await h.page.textContent("#progress-label")).toLowerCase().includes("finishing")
  );

  await h.emit("trim:progress", { fraction: 0.98, eta_seconds: 1 });
  await h.page.waitForTimeout(100);
  ok("near the end it stops pretending to be precise", (await h.page.textContent("#progress-label")).includes("almost done"));

  // No estimate yet is not a reason to show a broken one.
  await h.emit("trim:progress", { fraction: 0.1, eta_seconds: null });
  await h.page.waitForTimeout(100);
  const bare = await h.page.textContent("#progress-label");
  ok("and says nothing rather than NaN before it can estimate", bare.includes("10%") && !/NaN|undefined|null/.test(bare));

  release();
  await held;
  await h.done();
}

async function stretchesToCoverAreMarkedByHand() {
  // Hide chat on its own still covers the whole clip. These exist so a clip is
  // not obliged to carry a black corner from beginning to end.
  const h = await open("trim.html", scenario());
  await ready(h.page);
  await h.page.waitForTimeout(200);
  eq("nothing is listed to start with", await h.page.evaluate(() => document.getElementById("found").hidden), true);

  await h.page.evaluate(() => { document.getElementById("video").currentTime = 3; });
  await h.page.waitForTimeout(200);
  await h.page.click("#cover-mark");
  ok("marking says what it is waiting for", (await h.page.textContent("#found-count")).includes("scrub to the end"));
  eq("and the button asks for the other end", await h.page.textContent("#cover-mark"), "…to here");

  await h.page.evaluate(() => { document.getElementById("video").currentTime = 8; });
  await h.page.waitForTimeout(200);
  await h.page.click("#cover-mark");
  await h.page.waitForTimeout(150);
  eq("a stretch is listed", await h.page.evaluate(() => document.querySelectorAll("#found-list li").length), 1);
  ok("with its times", (await h.page.textContent("#found-list")).includes("0:03"));

  // The box is only over the picture while the playhead is inside the stretch.
  await h.page.evaluate(() => { document.getElementById("video").currentTime = 5; });
  await h.page.waitForTimeout(250);
  eq("covered inside the stretch", await h.page.evaluate(() => document.querySelectorAll(".found-box").length), 1);
  await h.page.evaluate(() => { document.getElementById("video").currentTime = 11; });
  await h.page.waitForTimeout(250);
  eq("and clear outside it", await h.page.evaluate(() => document.querySelectorAll(".found-box").length), 0);

  await h.page.evaluate(() => { document.getElementById("video").currentTime = 12; });
  await h.page.waitForTimeout(200);
  await h.page.click("#set-end");
  await h.page.click("#save");
  await h.page.waitForTimeout(500);
  const sent = (await h.lastCall("trim_clip"))?.args?.blackouts ?? [];
  eq("only that stretch is sent", sent.length, 1);
  ok("carrying the chat region", sent[0]?.w === 0.34);
  await h.done();
}

async function withNoStretchesTheWholeClipIsCovered() {
  // The old behaviour, and still the default: ticking Hide chat and marking
  // nothing must cover the corner throughout, not nothing at all.
  const h = await open("trim.html", scenario());
  await ready(h.page);
  await h.page.evaluate(() => { document.getElementById("video").currentTime = 12; });
  await h.page.waitForTimeout(200);
  await h.page.click("#set-end");
  await h.page.click("#save");
  await h.page.waitForTimeout(500);
  const args = (await h.lastCall("trim_clip"))?.args ?? {};
  eq("no stretches are sent", args.blackouts, null);
  eq("and hide chat is still asked for", args.hideChat, true);
  await h.done();
}

async function aFinishedTrimSaysSoBeforeItCloses() {
  // The window closing was the only success signal, so anything that stopped
  // it closing left the bar on "Finishing the file…" over a clip that was
  // already saved and already in the library.
  const h = await open("trim.html", scenario({
    trim_clip: "C:\\FiveMClip\\Clips\\Clip_2026_trimmed.mp4",
    chat_hiding: { available: false, on: false, region: null, reason: "" },
  }));
  await ready(h.page);
  await h.page.evaluate(() => { document.getElementById("video").currentTime = 12; });
  await h.page.waitForTimeout(200);
  await h.page.click("#set-end");
  await h.page.click("#save");

  // Before the close, not after: the point is that it is visible.
  await h.page.waitForFunction(
    () => /saved/i.test(document.getElementById("progress-label").textContent),
    null,
    { timeout: 4000 }
  ).catch(() => {});
  const label = await h.page.textContent("#progress-label");
  ok("it says it saved", /saved/i.test(label));
  ok("and names the file", label.includes("Clip_2026_trimmed.mp4"));
  eq("the bar is full", await h.page.evaluate(() => document.getElementById("progress-fill").style.width), "100%");

  await h.page.waitForTimeout(1200);
  eq("then it closes", await h.closed() > 0, true);
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

async function aLongSaveSaysSoRatherThanLookingHung() {
  // Closing the game with a session running holds the app for as long as
  // ffmpeg takes to stitch it. Windows paints that "Not responding"; the least
  // the app can do is say what it is waiting on.
  const h = await open("index.html", settingsScenario());
  await h.page.waitForTimeout(600);
  eq("nothing is claimed while idle", await h.page.evaluate(() => document.getElementById("busy-banner").hidden), true);

  await h.page.evaluate(() =>
    window.renderStatus({
      running: false, seconds_buffered: 0, pipeline: "nvenc-d3d11", has_audio: true,
      warnings: [], session_active: false, session_seconds: 0, session_bytes: 0,
      session_markers: 0, ffmpeg_found: true, fivem_running: false,
      estimated_buffer_bytes: 0, library_bytes: 0, free_bytes: 1e11,
      space: "Fine", paused_for_disk: false, portable: false, version: "test",
      busy: "Saving your session - this can take a minute on a long one.",
    })
  );
  eq("the banner appears while saving", await h.page.evaluate(() => document.getElementById("busy-banner").hidden), false);
  ok("and says what is happening", (await h.page.textContent("#busy-text")).includes("Saving your session"));
  eq("the status line agrees", await h.page.textContent("#status-text"), "Saving…");
  await h.done();
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

async function noPageHasTwoElementsWithTheSameId() {
  // A duplicate id is not a tidiness problem. The region overlay had two
  // #hint elements, so one of them collected both rules: `top` from one and
  // `bottom` from the other stretched it the full height of the screen, and
  // `border-radius: 999px` made it a pill - a tall dark bar down the middle of
  // every capture, and of anything recorded while one was open. It also breaks
  // getElementById, which only ever returns the first.
  for (const page of ["index.html", "trim.html", "region.html", "editor.html"]) {
    const h = await open(page, settingsScenario());
    await h.page.waitForTimeout(300);
    const dupes = await h.page.evaluate(() => {
      const seen = new Map();
      for (const el of document.querySelectorAll("[id]")) {
        seen.set(el.id, (seen.get(el.id) ?? 0) + 1);
      }
      return [...seen].filter(([, n]) => n > 1).map(([id, n]) => `${id} x${n}`);
    });
    eq(`${page} has no duplicate ids`, dupes.join(", "), "");
    await h.done();
  }
}

async function everyElementMainJsReachesForExists() {
  // Regrouping the settings markup dropped three fields on the floor - the
  // version note, the Saved badge and the whole chat-rules input - and the
  // first sign was a null dereference deep in collectSettings. Ask the page
  // directly instead: every id the script looks up has to be in the document.
  const h = await open("index.html", settingsScenario());
  await h.page.waitForTimeout(600);

  const source = await h.page.evaluate(async () => {
    const res = await fetch("main.js");
    return res.text();
  });
  const wanted = new Set();
  for (const m of source.matchAll(/\$\("([\w-]+)"\)/g)) wanted.add(m[1]);
  for (const m of source.matchAll(/getElementById\("([\w-]+)"\)/g)) wanted.add(m[1]);

  const missing = await h.page.evaluate(
    (ids) => ids.filter((id) => !document.getElementById(id)),
    [...wanted]
  );
  eq(`all ${wanted.size} ids main.js uses are in the page`, missing.join(", "), "");
  await h.done();
}

async function settingsAreBrokenIntoSections() {
  const h = await open("index.html", settingsScenario());
  await h.page.waitForTimeout(600);
  await h.page.click('.tab[data-view="settings"]');
  await h.page.waitForTimeout(200);

  eq("one section shows at a time", await h.page.evaluate(() =>
    [...document.querySelectorAll(".group[data-group]")].filter((g) => !g.hidden).length), 1);
  eq("and it is a handful of controls, not eighty", await h.page.evaluate(() =>
    [...document.querySelectorAll(".group[data-group]")]
      .find((g) => !g.hidden)
      .querySelectorAll("input, select, button").length) <= 30, true);

  await h.page.click('.subnav .chip[data-group="app"]');
  await h.page.waitForTimeout(150);
  eq("switching shows the one asked for", await h.page.evaluate(() =>
    document.querySelector(".group[data-group='app']").hidden), false);
  eq("and hides the rest", await h.page.evaluate(() =>
    document.querySelector(".group[data-group='recording']").hidden), true);

  // The one that would quietly destroy data: fields in a hidden section must
  // still be collected, or saving from Hotkeys blanks the S3 keys.
  const collected = await h.page.evaluate(() => collectSettings());
  ok("a hidden section is still collected", collected.hotkey_save_clip === "Ctrl+F5");
  ok("including its text fields", typeof collected.output_dir === "string" && collected.output_dir.length > 0);
  await h.done();
}

async function theSectionsFitTheSmallestWindow() {
  const h = await open("index.html", settingsScenario());
  await h.page.setViewportSize({ width: 860, height: 580 });
  await h.page.waitForTimeout(600);
  await h.page.click('.tab[data-view="settings"]');
  await h.page.waitForTimeout(200);

  // A switcher that runs off the edge hides sections nobody then knows exist.
  eq("every section is reachable at 860px", await h.page.evaluate(() =>
    [...document.querySelectorAll(".subnav .chip")].every((c) => {
      const b = c.getBoundingClientRect();
      return b.left >= 0 && b.right <= window.innerWidth + 1;
    })), true);
  eq("and nothing scrolls sideways", await h.page.evaluate(() =>
    document.documentElement.scrollWidth <= window.innerWidth + 1), true);
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
    theWholeClipCanStillHaveItsChatHidden,
    fastStaysOffAcrossADrag,
    draggingAHandleEnablesSaving,
    aFailedTrimLeavesTheWindowUsable,
    theSourceIsReleasedBeforeItIsReplaced,
    stretchesToCoverAreMarkedByHand,
    withNoStretchesTheWholeClipIsCovered,
    aTrimShowsHowFarAlongItIs,
    aFinishedTrimSaysSoBeforeItCloses,
    aLongSaveSaysSoRatherThanLookingHung,
    gameKeyHotkeysAreFlagged,
    theDefaultHotkeysAreClean,
    noPageHasTwoElementsWithTheSameId,
    everyElementMainJsReachesForExists,
    settingsAreBrokenIntoSections,
    theSectionsFitTheSmallestWindow,
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
