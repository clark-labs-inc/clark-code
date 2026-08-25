// Visual probe for diff rendering alignment. Loads the real app from the Vite
// dev server and mounts the REAL DiffBody/WorkLine components (app/src/probe/
// DiffProbeMount.tsx) so parseDiff + Shiki highlighting run exactly as shipped.
import { chromium } from "playwright";
import { existsSync } from "node:fs";

const systemChromium = [
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/Applications/Chromium.app/Contents/MacOS/Chromium",
].find((c) => existsSync(c));

const out = process.env.PROBE_OUT || "/tmp/diff-alignment.png";

const browser = await chromium.launch({ headless: true, executablePath: systemChromium });
const page = await browser.newPage({ viewport: { width: 760, height: 1100 }, deviceScaleFactor: 2 });
await page.goto("http://localhost:1420/", { waitUntil: "domcontentloaded" });
await page.waitForTimeout(2500);
await page.evaluate(async () => {
  const mod = await import("/src/probe/DiffProbeMount.tsx");
  document.documentElement.classList.add("dark");
  document.body.replaceChildren();
  document.body.appendChild(document.createElement("div"));
  mod.mountProbe(document.body.firstElementChild);
});
// Shiki highlights only after DIFF_HIGHLIGHT_QUIET_MS of quiet; wait it out.
await page.waitForTimeout(1500);
const diag = await page.evaluate(() => {
  const body = document.querySelector(".diff-body");
  const row = document.querySelector(".diff-row");
  const cs = row ? getComputedStyle(row) : null;
  return {
    url: location.href,
    hasBody: !!body,
    inlineVar: body ? body.style.getPropertyValue("--diff-gutter-ch") : null,
    columns: cs?.gridTemplateColumns ?? null,
    paddingLeft: cs?.paddingLeft ?? null,
    rows: document.querySelectorAll(".diff-row").length,
    worklines: document.querySelectorAll("[data-tool-call-id]").length,
  };
});
console.log(JSON.stringify(diag, null, 2));
await page.screenshot({ path: out, fullPage: true });
await browser.close();
console.log(`saved ${out}`);
