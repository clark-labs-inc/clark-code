// Record a real Clark turn through the UI to a webm in ../videos/.
// Usage: node record.mjs "<query>" <provider> <outName> [maxWaitSec] [theme]
import { chromium } from "playwright";
import { EXECUTABLE, VIEWPORT } from "./launch.mjs";
import { rename } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const VIDEOS = join(ROOT, "videos");

const query = process.argv[2] ?? "In one sentence, what is Rust?";
const provider = process.argv[3] ?? "clark";
const outName = process.argv[4] ?? "clip";
const maxWaitSec = parseInt(process.argv[5] ?? "180", 10);
const theme = process.argv[6] ?? "light";

const url =
  `http://localhost:1420/?dev&provider=${provider}&q=` + encodeURIComponent(query);

const browser = await chromium.launch({ headless: true, executablePath: EXECUTABLE });
const context = await browser.newContext({
  viewport: VIEWPORT,
  recordVideo: { dir: VIDEOS, size: VIEWPORT },
  deviceScaleFactor: 2,
});
const page = await context.newPage();
if (theme === "dark") {
  await page.addInitScript(() => localStorage.setItem("clark.theme", "dark"));
}
// Clark gateway creds (only needed for the Clark provider) come from the
// environment — CLARK_WS / CLARK_TOKEN — never hardcoded.
if (process.env.CLARK_WS || process.env.CLARK_TOKEN) {
  await page.addInitScript(
    ([ep, tok]) => {
      if (ep) localStorage.setItem("clark.endpoint", ep);
      if (tok) localStorage.setItem("clark.token", tok);
    },
    [process.env.CLARK_WS, process.env.CLARK_TOKEN],
  );
}
await page.goto(url, { waitUntil: "domcontentloaded" });

const stop = page.locator('[aria-label="Stop"]');
// Wait for the run to start (Stop button appears), then finish (it disappears).
try {
  await stop.waitFor({ state: "visible", timeout: 30000 });
  console.log("run started…");
  await stop.waitFor({ state: "hidden", timeout: maxWaitSec * 1000 });
  console.log("run finished.");
} catch (e) {
  console.log("wait note:", String(e).split("\n")[0]);
}
await page.waitForTimeout(800);

// Demonstrate expandable work: open a work line (not the plan header).
const expandable = page
  .locator('button[aria-expanded="false"]:not([disabled])')
  .filter({ hasText: /Read|Edit|Write|Search|Run|Open|Fetch/ })
  .first();
if (await expandable.count()) {
  await expandable.scrollIntoViewIfNeeded();
  await expandable.click().catch(() => {});
  await page.waitForTimeout(1600);
}
await page.waitForTimeout(600);

const video = page.video();
await context.close();
await browser.close();
if (video) {
  const src = await video.path();
  const dest = join(VIDEOS, `${outName}.webm`);
  await rename(src, dest);
  console.log("VIDEO:", dest);
}
