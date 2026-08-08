// Browser probe: the left conversation sidebar is horizontally resizable.
//
// Starts the dev server with the deterministic dev auth, then drives a real
// Chromium page: drag the separator, nudge with arrow keys, verify clamping,
// persistence across reload, and that collapsing/expanding keeps the saved
// width. Mirrors the style of selection-repro.mjs / motion-exit-probe.mjs.

import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { setTimeout as sleep } from "node:timers/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { launch, VIEWPORT } from "./launch.mjs";

const repoDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function reservePort() {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = typeof address === "object" && address ? address.port : null;
      server.close((error) => error ? reject(error) : resolve(port));
    });
  });
}

function check(condition, message) {
  if (!condition) throw new Error(message);
  console.log(`ok: ${message}`);
}

async function approveIfNeeded(page) {
  const button = page.getByRole("button", { name: "Allow once" });
  try {
    await button.waitFor({ state: "visible", timeout: 1_500 });
    await button.click();
  } catch (error) {
    if (!String(error?.message ?? error).includes("Timeout")) throw error;
  }
}

const port = await reservePort();
const url = `http://127.0.0.1:${port}/`;
const dev = spawn("pnpm", ["--dir", "app", "dev", "--host", "127.0.0.1", "--port", String(port), "--strictPort"], {
  cwd: repoDir,
  env: { ...process.env, VITE_PRODUCT_DEV_AUTH: "1" },
  stdio: ["ignore", "pipe", "pipe"],
});
let serverOutput = "";
dev.stdout.on("data", (chunk) => { serverOutput += chunk; });
dev.stderr.on("data", (chunk) => { serverOutput += chunk; });

async function waitForServer() {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (dev.exitCode != null) throw new Error(`Vite exited early\n${serverOutput}`);
    try {
      if ((await fetch(url)).ok) return;
    } catch {}
    await sleep(100);
  }
  throw new Error(`Vite did not start\n${serverOutput}`);
}

async function asideWidth(page) {
  return page.locator("aside").first().evaluate((el) => Math.round(el.getBoundingClientRect().width));
}
async function storedWidth(page) {
  return page.evaluate(() => Number(localStorage.getItem("agent-desktop.sidebar-width")));
}

let browser;
try {
  await waitForServer();
  browser = await launch();
  const context = await browser.newContext({ viewport: VIEWPORT });
  await context.addInitScript(() => {
    const accountScope = "id:full-gui-qa";
    const encodedScope = encodeURIComponent(accountScope);
    localStorage.setItem("agent-desktop.dev-account", JSON.stringify({
      user: { id: "full-gui-qa", name: "Full GUI QA", method: "local" },
    }));
    localStorage.setItem(`agent-desktop:local-agent:${encodedScope}`, JSON.stringify({
      cwd: "/tmp", model: "local-model", reasoningEffort: "high",
    }));
    localStorage.setItem(`agent-desktop:project-context:${encodedScope}`, JSON.stringify({ cwd: "/tmp" }));
  });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.stack ?? error.message));
  page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
  await page.goto(url, { waitUntil: "networkidle" });
  const separator = page.getByRole("separator", { name: "Resize sidebar" });
  await separator.waitFor({ state: "visible" });
  await approveIfNeeded(page);

  // 1. Starts at the historical 17rem (272px).
  check(await asideWidth(page) === 272, "starts at 272px (17rem)");

  // 2. Drag the separator right by ~120px → the sidebar grows accordingly.
  const box = await separator.boundingBox();
  check(Boolean(box && box.width > 0), "separator is an 8px-wide hit surface");
  const startX = box.x + box.width / 2;
  const startY = box.y + box.height / 2;
  await page.mouse.move(startX, startY);
  await page.mouse.down();
  await page.mouse.move(startX + 120, startY, { steps: 8 });
  await page.mouse.up();
  const dragged = await asideWidth(page);
  check(Math.abs(dragged - Math.round(startX + 120)) <= 2, `drag +120px → ${dragged}px (clientX ${Math.round(startX + 120)})`);
  check(Math.abs(await storedWidth(page) - dragged) <= 1, "width persisted on drag end");

  // 3. Arrow keys nudge by 24px.
  await separator.focus();
  await page.keyboard.press("ArrowRight");
  check(await asideWidth(page) === 412, "ArrowRight nudges +24px (→ 412)");
  await page.keyboard.press("ArrowRight");
  check(await asideWidth(page) === 436, "second ArrowRight nudges again (→ 436)");

  // 4. Overscrolling clamps: drag from the separator's new position far past
  // the maximum (640 on this viewport).
  const wideBox = await separator.boundingBox();
  const wideX = wideBox.x + wideBox.width / 2;
  await page.mouse.move(wideX, wideBox.y + wideBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(wideX + 2500, wideBox.y + wideBox.height / 2, { steps: 12 });
  await page.mouse.up();
  check(await asideWidth(page) === 640, "huge drag clamps to 640px");

  // 5. Home → minimum, End → maximum.
  await separator.focus();
  await page.keyboard.press("Home");
  check(await asideWidth(page) === 200, "Home → 200px minimum");
  await page.keyboard.press("End");
  check(await asideWidth(page) === 640, "End → 640px maximum");

  // 6. Persists across a reload.
  await page.reload({ waitUntil: "networkidle" });
  await separator.waitFor({ state: "visible" });
  check(await asideWidth(page) === 640, "reload restores the saved 640px");
  check(Math.abs(await storedWidth(page) - 640) <= 1, "localStorage holds 640");

  // 7. Collapse to the rail and back keeps the saved width.
  await page.getByRole("button", { name: "Collapse sidebar" }).click();
  await page.getByRole("button", { name: "Expand sidebar" }).waitFor({ state: "visible" });
  await page.getByRole("button", { name: "Expand sidebar" }).click();
  await separator.waitFor({ state: "visible" });
  check(await asideWidth(page) === 640, "collapse/expand restores the saved width");

  // 8. Double-click resets to the default and persists it.
  await separator.dblclick();
  await page.waitForTimeout(100);
  const afterDbl = await asideWidth(page);
  check(afterDbl === 272, `double-click resets to 272px (was ${afterDbl})`);
  const storedAfterDbl = await storedWidth(page);
  check(Math.abs(storedAfterDbl - 272) <= 1, `reset width persisted (stored ${storedAfterDbl})`);

  check(errors.length === 0, `no page errors (${errors.length})`);
  console.log("sidebar-resize-probe: PASS");
} finally {
  if (browser) await browser.close();
  dev.kill("SIGTERM");
}