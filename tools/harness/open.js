// Boot a page from ui/ with a stubbed backend, ready to drive.
const fs = require("fs");
const path = require("path");
const { chromium } = require("playwright");
const { serve } = require("./serve");
const { install, payload } = require("./stub");
const { UI, MEDIA } = require("./paths");

/** Playwright wants the exact Chromium revision it was built against, which a
 *  preinstalled browser directory will not always have. Prefer whatever is
 *  actually on disk over a download that is disabled here anyway. */
function chromePath() {
  if (process.env.FIVEMCLIP_HARNESS_CHROME) return process.env.FIVEMCLIP_HARNESS_CHROME;
  const root = process.env.PLAYWRIGHT_BROWSERS_PATH;
  if (!root || !fs.existsSync(root)) return undefined;
  for (const dir of fs.readdirSync(root).filter((d) => d.startsWith("chromium-"))) {
    for (const rel of ["chrome-linux/chrome", "chrome-linux64/chrome", "chrome-win/chrome.exe"]) {
      const candidate = path.join(root, dir, rel);
      if (fs.existsSync(candidate)) return candidate;
    }
  }
  return undefined;
}

async function open(page_name, scenario = {}) {
  const { port, server } = await serve({ "/": UI, "/media": MEDIA });
  const browser = await chromium.launch({ executablePath: chromePath() });
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });

  const problems = [];
  page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
  page.on("console", (m) => {
    if (m.type() === "error") problems.push(`console: ${m.text()}`);
  });

  await page.addInitScript(
    install,
    payload({ ...scenario, assetBase: `http://127.0.0.1:${port}/media` })
  );
  await page.goto(`http://127.0.0.1:${port}/${page_name}`);

  return {
    page,
    problems,
    calls: () => page.evaluate(() => window.__harness.calls),
    lastCall: (name) => page.evaluate((n) => window.__harness.lastCall(n), name),
    alerts: () => page.evaluate(() => window.__harness.alerts),
    closed: () => page.evaluate(() => window.__harness.closed),
    emit: (name, value) =>
      page.evaluate(([n, v]) => window.__harness.emit(n, v), [name, value]),
    async done() {
      await browser.close();
      server.close();
    },
  };
}

module.exports = { open };
