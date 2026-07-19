// First-hand check of the xterm.js selection behavior reported upstream:
// select text in the terminal, click an empty page area, observe whether the
// terminal selection survives. Runs the page twice — plain and with the
// proposed "clear on outside mousedown" fix (?fixed) — and asserts the fix.
//
// Usage: node harness/xterm-selection-repro.mjs
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { join, extname } from "node:path";
import { fileURLToPath } from "node:url";
import { webkit, chromium } from "playwright";

const root = fileURLToPath(new URL("..", import.meta.url));
const MIME = { ".html": "text/html", ".mjs": "text/javascript", ".js": "text/javascript", ".css": "text/css", ".map": "application/json" };

const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url, "http://x");
    let path = url.pathname === "/" ? "/xterm-test/index.html" : url.pathname;
    let file;
    if (path.startsWith("/@xterm/")) file = join(root, "app/node_modules", path);
    else file = join(root, "harness", path);
    const body = await readFile(file);
    res.writeHead(200, { "content-type": MIME[extname(file)] ?? "application/octet-stream" });
    res.end(body);
  } catch (e) {
    res.writeHead(404).end(String(e));
  }
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}/`;

async function run(browserType, name, fixed) {
  const browser = await browserType.launch({
    headless: true,
    executablePath:
      browserType === chromium ? process.env.PLAYWRIGHT_CHROMIUM_PATH || undefined : undefined,
  });
  const page = await browser.newPage({ viewport: { width: 900, height: 700 } });
  await page.goto(base + (fixed ? "?fixed" : ""), { waitUntil: "networkidle" });
  await page.waitForFunction(() => window.term?.element != null);
  await page.waitForTimeout(300);

  // Drag-select across the middle of the terminal.
  const box = await page.locator("#termwrap").boundingBox();
  await page.mouse.move(box.x + 30, box.y + 60);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width - 60, box.y + 120, { steps: 10 });
  await page.mouse.up();
  await page.waitForTimeout(150);
  const afterSelect = await page.evaluate(() => ({
    has: window.term.hasSelection(),
    sel: window.term.getSelection().slice(0, 50),
  }));

  // Click the empty page area above the terminal.
  await page.mouse.click(450, 150);
  await page.waitForTimeout(150);
  const afterOutsideClick = await page.evaluate(() => window.term.hasSelection());

  // Sanity: can still select inside the terminal afterwards.
  await page.mouse.move(box.x + 30, box.y + 60);
  await page.mouse.down();
  await page.mouse.move(box.x + 200, box.y + 60, { steps: 6 });
  await page.mouse.up();
  await page.waitForTimeout(120);
  const reselect = await page.evaluate(() => window.term.hasSelection());

  console.log(
    `[${name}${fixed ? " +fix" : ""}] selected=${afterSelect.has} ("${afterSelect.sel}…") ` +
      `afterOutsideClick=${afterOutsideClick} reselectInside=${reselect}`,
  );
  await browser.close();
  return { afterSelect: afterSelect.has, afterOutsideClick, reselect };
}

try {
  for (const [engine, name] of [[webkit, "webkit"], [chromium, "chromium"]]) {
    const plain = await run(engine, name, false);
    const fixed = await run(engine, name, true);
    const ok =
      plain.afterSelect && plain.afterOutsideClick === true && // bug reproduces
      fixed.afterSelect && fixed.afterOutsideClick === false && // fix clears it
      fixed.reselect; // terminal still usable
    console.log(`${name}: ${ok ? "PASS" : "FAIL"}`);
    if (!ok) process.exitCode = 1;
  }
} finally {
  server.close();
}
