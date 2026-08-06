#!/usr/bin/env node

// Deterministic specialist GUI contract. It exercises the rendered browser
// boundary only: no provider, model, cloud mutation, or specialist run is
// started. The same stable data-qa selectors are exposed to Clark Tester when
// it drives a WebKit/macOS surface through accessibility or Appium.

import assert from "node:assert/strict";
import { createServer } from "node:net";
import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";

import { launch, VIEWPORT } from "./launch.mjs";

const repoDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const kinds = ["scout", "security", "scientist", "rsi"];
const labels = {
  scout: "Scout",
  security: "Security",
  scientist: "Scientist",
  rsi: "RSI",
};

function parseOut(args) {
  const index = args.indexOf("--out");
  const inline = args.find((arg) => arg.startsWith("--out="));
  const value = inline ? inline.slice("--out=".length) : index >= 0 ? args[index + 1] : null;
  if (index >= 0 && (!value || value.startsWith("--"))) throw new Error("--out requires a value");
  return value;
}

async function availablePort() {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = typeof address === "object" && address ? address.port : null;
      server.close((error) => (error ? reject(error) : resolve(port)));
    });
  });
}

function outputDirectory(requested) {
  const targetRoot = path.join(repoDir, "target");
  const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
  const resolved = path.resolve(
    requested ?? path.join(targetRoot, "specialist-ui-smoke", `${stamp}-${process.pid}`),
  );
  if (resolved === targetRoot || !resolved.startsWith(`${targetRoot}${path.sep}`)) {
    throw new Error("specialist UI output must stay under repository target");
  }
  if (existsSync(resolved)) throw new Error(`refusing to overwrite ${resolved}`);
  mkdirSync(resolved, { recursive: true, mode: 0o700 });
  return resolved;
}

function seedPreview() {
  localStorage.clear();
  const accountScope = "id:specialist-ui-qa";
  const encodedScope = encodeURIComponent(accountScope);
  localStorage.setItem(
    "clark.desktop.dev-account",
    JSON.stringify({
      user: {
        id: "specialist-ui-qa",
        name: "Specialist UI QA",
        email: "specialist-ui-qa@clark.local",
        method: "local",
      },
    }),
  );
  localStorage.setItem(
    `clark-desktop:local-agent:${encodedScope}`,
    JSON.stringify({ cwd: "", model: "clark-code:free", reasoningEffort: "high" }),
  );
  localStorage.setItem(
    `clark-desktop:project-context:${encodedScope}`,
    JSON.stringify({ cwd: "/tmp/clark-specialist-ui-qa" }),
  );
}

async function waitForServer(url, processHandle, log) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (processHandle.exitCode != null) throw new Error(`Vite exited early\n${log()}`);
    try {
      if ((await fetch(url)).ok) return;
    } catch {
      // The dev server is still starting.
    }
    await sleep(150);
  }
  throw new Error(`Vite did not start\n${log()}`);
}

async function assertVisible(locator, message) {
  await locator.waitFor({ state: "visible", timeout: 15_000 });
  assert.equal(await locator.isVisible(), true, message);
}

function observePageErrors(page, errors) {
  page.on("pageerror", (error) => errors.push(error.stack ?? error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
}

async function isolatedPage(errors) {
  const browser = await launch();
  const context = await browser.newContext({ viewport: VIEWPORT });
  await context.addInitScript(seedPreview);
  const page = await context.newPage();
  observePageErrors(page, errors);
  return { page, close: () => browser.close() };
}

async function waitForVisualStability(page, locator) {
  await page.evaluate(async () => {
    await document.fonts.ready;
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  });
  let previous = null;
  let stableFrames = 0;
  for (let attempt = 0; attempt < 10; attempt += 1) {
    const box = await locator.boundingBox();
    assert.ok(box, "visible specialist heading has no layout box");
    const current = [box.x, box.y, box.width, box.height].map((value) => Math.round(value * 10) / 10);
    stableFrames = previous && current.every((value, index) => value === previous[index])
      ? stableFrames + 1
      : 0;
    if (stableFrames >= 2) return box;
    previous = current;
    await page.waitForTimeout(80);
  }
  throw new Error("specialist heading did not reach a visually stable layout");
}

async function assertHeadingClearsSidebar(page, heading) {
  const headingBox = await waitForVisualStability(page, heading);
  const navigationBox = await page.locator('[data-qa="specialist-navigation"]').boundingBox();
  assert.ok(navigationBox, "specialist navigation has no layout box");
  assert.ok(
    headingBox.x >= navigationBox.x + navigationBox.width,
    `specialist heading overlaps navigation: heading x=${headingBox.x}, navigation right=${navigationBox.x + navigationBox.width}`,
  );
}

async function runPaid(errors, baseUrl, outputDir) {
  const result = { mode: "paid_preview", specialists: {} };

  for (const kind of kinds) {
    const { page, close } = await isolatedPage(errors);
    try {
      await page.goto(`${baseUrl}?specialistPreview=paid`, { waitUntil: "domcontentloaded" });
      await assertVisible(page.locator('[data-qa="specialist-navigation"]'), "specialist navigation missing");
      const nav = page.locator(`[data-qa="specialist-nav-${kind}"]`).last();
      await assertVisible(nav, `${kind} navigation missing`);
      await nav.click();
      await assertVisible(page.locator(`[data-qa="specialist-workspace-${kind}"]`), `${kind} workspace missing`);
      const heading = page.getByRole("heading", { name: `Clark ${labels[kind]}`, exact: true });
      await assertVisible(heading, `${kind} heading missing`);
      await assertHeadingClearsSidebar(page, heading);
      await assertVisible(page.getByText("Access ready", { exact: true }), `${kind} access state missing`);

      const welcome = page.locator(`[data-qa="specialist-welcome-${kind}"]`);
      await assertVisible(welcome, `${kind} welcome missing`);
      assert.equal(
        await page.locator(`[data-qa^="specialist-starter-${kind}-"]`).count(),
        3,
        `${kind} starter count changed`,
      );

      await page.locator(`[data-qa="specialist-intro-${kind}-example"]`).click();
      await assertVisible(welcome.getByText("Demo data · no work has run", { exact: true }), `${kind} example missing`);
      await page.locator(`[data-qa="specialist-intro-${kind}-start"]`).click();

      const starter = page.locator(`[data-qa="specialist-starter-${kind}-0"]`);
      await starter.click();
      const composer = page.getByLabel("Message Clark");
      await assertVisible(composer, `${kind} composer missing`);
      assert.ok((await composer.inputValue()).trim(), `${kind} starter did not prefill composer`);

      const canvas = page.locator(`[data-qa="specialist-canvas-${kind}"]`);
      const toggle = page.locator(`[data-qa="specialist-show-insights-${kind}"]`);
      if (!(await canvas.isVisible())) await toggle.click();
      await assertVisible(canvas, `${kind} canvas missing`);
      const canvasText = (await canvas.innerText()).replace(/\s+/g, " ").trim();
      assert.ok(canvasText.length >= 40, `${kind} canvas has no representative content`);
      await page.waitForTimeout(300);
      await assertHeadingClearsSidebar(page, heading);

      const screenshot = path.join(outputDir, `${kind}-paid.png`);
      await page.locator("#root").screenshot({ path: screenshot, animations: "disabled" });
      result.specialists[kind] = {
        heading: `Clark ${labels[kind]}`,
        starters: 3,
        example: true,
        starter_prefill: true,
        canvas: true,
        screenshot,
      };
    } finally {
      await close();
    }
  }
  return result;
}

async function runFree(errors, baseUrl, outputDir) {
  const result = { mode: "free_preview", specialists: {} };
  for (const kind of kinds) {
    const { page, close } = await isolatedPage(errors);
    try {
      await page.goto(`${baseUrl}?specialistPreview=free`, { waitUntil: "domcontentloaded" });
      await assertVisible(page.locator('[data-qa="specialist-navigation"]'), "specialist navigation missing in free preview");
      await page.locator(`[data-qa="specialist-nav-${kind}"]`).last().click();
      const gate = page.locator(`[data-qa="specialist-gate-${kind}"]`);
      await assertVisible(gate, `${kind} free gate missing`);
      assert.match(await gate.innerText(), /unlock Clark|Pro coverage/i, `${kind} free gate copy missing`);
      await assertVisible(gate.getByRole("button", { name: "Compare plans", exact: true }), `${kind} upgrade action missing`);
      const heading = page.getByRole("heading", { name: `Clark ${labels[kind]}`, exact: true });
      await assertVisible(heading, `${kind} free heading missing`);
      await assertHeadingClearsSidebar(page, heading);
      await page.waitForTimeout(300);
      const screenshot = path.join(outputDir, `${kind}-free.png`);
      await page.locator("#root").screenshot({ path: screenshot, animations: "disabled" });
      result.specialists[kind] = { gated: true, screenshot };
    } finally {
      await close();
    }
  }
  return result;
}

async function runResponsive(errors, baseUrl, outputDir) {
  const { page, close } = await isolatedPage(errors);
  try {
    await page.goto(`${baseUrl}?specialistPreview=paid`, { waitUntil: "domcontentloaded" });
    await page.setViewportSize({ width: 375, height: 812 });
    await page.locator('[data-qa="specialist-nav-scientist"]').last().click();
    await assertVisible(page.locator('[data-qa="specialist-workspace-scientist"]'), "mobile specialist workspace missing");
    await assertVisible(page.locator('[data-qa="specialist-welcome-scientist"]'), "mobile specialist welcome missing");
    const metrics = await page.evaluate(() => ({
      innerWidth: window.innerWidth,
      scrollWidth: document.documentElement.scrollWidth,
      scrollHeight: document.documentElement.scrollHeight,
    }));
    assert.ok(metrics.scrollWidth <= metrics.innerWidth, "mobile layout has document-level horizontal overflow");
    const screenshot = path.join(outputDir, "responsive-mobile.png");
    await page.locator("#root").screenshot({ path: screenshot, animations: "disabled" });
    return { viewport: { width: 375, height: 812 }, metrics, no_document_overflow: true, screenshot };
  } finally {
    await close();
  }
}

const outputDir = outputDirectory(parseOut(process.argv.slice(2)));
const port = Number(process.env.CLARK_SPECIALIST_UI_PORT ?? await availablePort());
const url = `http://127.0.0.1:${port}/`;
const dev = spawn(
  "pnpm",
  ["--dir", "app", "dev", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
  {
    cwd: repoDir,
    env: { ...process.env, VITE_CLARK_DEV_AUTH: "1" },
    stdio: ["ignore", "pipe", "pipe"],
  },
);
let devLog = "";
dev.stdout.on("data", (chunk) => (devLog += chunk));
dev.stderr.on("data", (chunk) => (devLog += chunk));

const receipt = {
  schema_version: 1,
  benchmark: "clark_desktop_specialist_ui_smoke",
  status: "failed",
  source_revision: null,
  source_dirty: null,
  paid_calls_made: false,
  viewport: VIEWPORT,
  output_dir: outputDir,
  browser_console_errors: [],
  failure: null,
};

try {
  receipt.source_revision = execFileSync("git", ["rev-parse", "HEAD"], { cwd: repoDir, encoding: "utf8" }).trim();
  receipt.source_dirty = Boolean(execFileSync("git", ["status", "--porcelain"], { cwd: repoDir, encoding: "utf8" }).trim());
  await waitForServer(url, dev, () => devLog);
  const errors = [];

  receipt.paid_preview = await runPaid(errors, url, outputDir);
  receipt.free_preview = await runFree(errors, url, outputDir);
  receipt.responsive = await runResponsive(errors, url, outputDir);
  receipt.browser_console_errors = errors;
  assert.deepEqual(errors, [], `browser errors: ${errors.join("\n")}`);
  receipt.status = "passed";
} catch (error) {
  receipt.failure = error instanceof Error ? error.message : String(error);
} finally {
  dev.kill("SIGTERM");
  writeFileSync(path.join(outputDir, "receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o600 });
}

console.log(JSON.stringify(receipt, null, 2));
if (receipt.status !== "passed") process.exitCode = 1;
