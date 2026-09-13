// Clips for the harness to open. Generated rather than committed - an 8 MB
// binary in the tree to test a seek bar is not a trade worth making.
//
// VP9/Opus in WebM, not H.264/AAC in MP4, and not by preference: Playwright's
// Chromium is built without the proprietary codecs, so an .mp4 fixture loads as
// "Could not open that clip" and every timeline test fails for a reason that
// has nothing to do with the code under test. WebView2 on Windows plays H.264
// perfectly well; the container is a property of the harness, and the page
// itself never looks at it.
const { execFileSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const { MEDIA } = require("./paths");

const FFMPEG = process.env.FIVEMCLIP_TEST_FFMPEG || "ffmpeg";

function make(name, args) {
  const out = path.join(MEDIA, name);
  if (fs.existsSync(out) && fs.statSync(out).size > 0) return out;
  fs.mkdirSync(MEDIA, { recursive: true });
  try {
    execFileSync(FFMPEG, ["-y", "-hide_banner", "-loglevel", "error", ...args, out]);
  } catch (e) {
    // A half-written file passes the existsSync check on the next run and then
    // fails as a corrupt fixture, which reads as a bug in the page.
    fs.rmSync(out, { force: true });
    throw e;
  }
  return out;
}

const SOURCE = [
  "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30:duration=20",
  "-f", "lavfi", "-i", "sine=frequency=440:duration=20",
  "-vf",
  "drawtext=text='CHAT LINE ONE':x=40:y=520:fontsize=28:fontcolor=white," +
    "drawtext=text='CHAT LINE TWO':x=40:y=560:fontsize=28:fontcolor=white",
  "-c:v", "libvpx-vp9", "-b:v", "1M", "-deadline", "realtime", "-cpu-used", "8",
  "-g", "60", "-pix_fmt", "yuv420p", "-c:a", "libopus",
];

/** 20s of 720p with two lines of "chat" in the bottom left, so a blackout has
 *  something to remove and a preview has something to sit over. */
const clip = () => make("clip.webm", SOURCE);

/** The same thing with no duration in its header, which is what a recording
 *  that was still being written looks like to a <video>: `duration` comes back
 *  Infinity until something makes the browser go and find the end. */
const clipWithoutDuration = () =>
  make("no-duration.webm", [...SOURCE, "-f", "webm", "-live", "1"]);

module.exports = { clip, clipWithoutDuration };
