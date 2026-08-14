// Deterministic browser smoke for Clark Code's Pragmatic Drag and Drop paths.
// Verifies pointer reordering, the equivalent menu action, desktop file drops,
// and the equivalent file picker without starting an agent/model run.

import { spawn } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";

import { launch, VIEWPORT } from "./launch.mjs";

const repoDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
const outDir = path.join(repoDir, "target", "pragmatic-dnd-smoke", `${stamp}-${process.pid}`);

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
}

const port = await reservePort();
const url = `http://127.0.0.1:${port}/`;
const dev = spawn(
  "pnpm",
  ["--dir", "app", "dev", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
  {
    cwd: repoDir,
    env: { ...process.env, VITE_PRODUCT_DEV_AUTH: "1" },
    stdio: ["ignore", "pipe", "pipe"],
  },
);
let serverOutput = "";
dev.stdout.on("data", (chunk) => { serverOutput += chunk; });
dev.stderr.on("data", (chunk) => { serverOutput += chunk; });

async function waitForServer() {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    if (dev.exitCode != null) throw new Error(`Vite exited early\n${serverOutput}`);
    try {
      if ((await fetch(url)).ok) return;
    } catch {}
    await sleep(100);
  }
  throw new Error(`Vite did not start\n${serverOutput}`);
}

async function storedPinnedOrder(page) {
  return page.evaluate(() => {
    const raw = localStorage.getItem("agent-desktop:project-sidebar:id%3Adnd-qa");
    return raw ? JSON.parse(raw).pinned : [];
  });
}

async function waitForPinnedOrder(page, expected) {
  await page.waitForFunction(
    ({ key, expectedOrder }) => {
      const raw = localStorage.getItem(key);
      const order = raw ? JSON.parse(raw).pinned : [];
      return JSON.stringify(order) === JSON.stringify(expectedOrder);
    },
    { key: "agent-desktop:project-sidebar:id%3Adnd-qa", expectedOrder: expected },
  );
}

let browser;
const errors = [];
try {
  await mkdir(outDir, { recursive: true });
  await waitForServer();
  browser = await launch();
  const context = await browser.newContext({ viewport: VIEWPORT });
  await context.addInitScript(() => {
    const accountScope = "id:dnd-qa";
    const encodedScope = encodeURIComponent(accountScope);
    localStorage.setItem("agent-desktop.dev-account", JSON.stringify({
      user: { id: "dnd-qa", name: "Drag and Drop QA", method: "local" },
    }));
    localStorage.setItem(`agent-desktop:local-agent:${encodedScope}`, JSON.stringify({
      cwd: "/tmp/alpha", model: "local-model", reasoningEffort: "high",
    }));
    localStorage.setItem(
      `agent-desktop:project-context:${encodedScope}`,
      JSON.stringify({ cwd: "/tmp/alpha" }),
    );
    localStorage.setItem(
      `agent-desktop:recent-projects:${encodedScope}`,
      JSON.stringify(["/tmp/alpha", "/tmp/bravo", "/tmp/charlie"]),
    );
    localStorage.setItem(
      `agent-desktop:project-sidebar:${encodedScope}`,
      JSON.stringify({
        pinned: ["p:/tmp/alpha", "p:/tmp/bravo", "p:/tmp/charlie"],
        aliases: {},
      }),
    );
  });

  const page = await context.newPage();
  page.on("pageerror", (error) => errors.push(error.stack ?? error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  await page.goto(url, { waitUntil: "domcontentloaded" });
  await page.getByLabel("Message Clark Code").waitFor({ state: "visible" });
  await page.locator('[data-sidebar-project="p:/tmp/alpha"]').waitFor({ state: "visible" });

  // Non-drag alternative: the existing Project actions menu can place a
  // pinned project at every position and returns focus to its trigger.
  const alphaActions = page.getByLabel("Project actions for alpha");
  await alphaActions.focus();
  await page.keyboard.press("Enter");
  const unpinItem = page.getByRole("menuitem", { name: "Unpin project" });
  await unpinItem.waitFor();
  check(await unpinItem.evaluate((element) => element === document.activeElement), "project menu focuses its first action");
  await page.keyboard.press("Tab");
  const moveItem = page.getByRole("menuitem", { name: "Move project…" });
  check(await moveItem.evaluate((element) => element === document.activeElement), "keyboard reaches move action");
  await page.keyboard.press("Enter");
  const backToActions = page.getByLabel("Back to project actions");
  await backToActions.waitFor();
  check(await backToActions.evaluate((element) => element === document.activeElement), "move destinations receive focus");
  await page.screenshot({
    path: path.join(outDir, "project-move-menu.png"),
    animations: "disabled",
  });
  await page.keyboard.press("Tab");
  await page.keyboard.press("Tab");
  const afterCharlie = page.getByRole("menuitem", { name: "After charlie" });
  check(await afterCharlie.evaluate((element) => element === document.activeElement), "keyboard reaches exact destination");
  await page.keyboard.press("Enter");
  await waitForPinnedOrder(page, ["p:/tmp/bravo", "p:/tmp/charlie", "p:/tmp/alpha"]);
  check(await alphaActions.evaluate((element) => element === document.activeElement), "move menu restores focus");

  // Pointer path: drag the same pinned project back before the first project.
  const alphaHandle = page.locator('[data-project-drag-handle="p:/tmp/alpha"]');
  const bravoTarget = page.locator('[data-sidebar-project="p:/tmp/bravo"]');
  const targetBox = await bravoTarget.boundingBox();
  check(Boolean(targetBox), "pinned project target has a layout box");
  await alphaHandle.dragTo(bravoTarget, {
    targetPosition: { x: Math.max(4, targetBox.width / 2), y: 2 },
  });
  await waitForPinnedOrder(page, ["p:/tmp/alpha", "p:/tmp/bravo", "p:/tmp/charlie"]);

  // External adapter path: dispatch a real DataTransfer file sequence onto the
  // composer and verify the canonical attachment chip.
  await page.locator('[data-file-drop-target="composer"]').evaluate((target) => {
    const transfer = new DataTransfer();
    transfer.items.add(new File(["pragmatic drop"], "pragmatic-drop.txt", { type: "text/plain" }));
    for (const type of ["dragenter", "dragover", "drop"]) {
      target.dispatchEvent(new DragEvent(type, {
        bubbles: true,
        cancelable: true,
        dataTransfer: transfer,
      }));
    }
  });
  await page.getByRole("listitem").filter({ hasText: "pragmatic-drop.txt" }).waitFor();

  // Non-drag alternative: the hidden file input behind Add attachments feeds
  // the exact same store and chip UI.
  await page.getByLabel("Add attachments").click();
  await page.getByRole("menuitem", { name: /^Files/ }).waitFor({ state: "visible" });
  await page.getByTestId("composer-file-input").setInputFiles({
    name: "picker-alternative.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("picker alternative"),
  });
  await page.getByRole("listitem").filter({ hasText: "picker-alternative.txt" }).waitFor();

  await page.screenshot({
    path: path.join(outDir, "pragmatic-dnd-complete.png"),
    animations: "disabled",
  });
  check(errors.length === 0, `browser errors:\n${errors.join("\n")}`);

  const receipt = {
    schema_version: 1,
    benchmark: "agent_desktop_pragmatic_dnd_smoke",
    status: "passed",
    mode: "ui_only_no_model_calls",
    viewport: VIEWPORT,
    checks: [
      "project_pointer_reorder",
      "project_keyboard_menu_reorder",
      "menu_focus_restoration",
      "external_file_drop",
      "file_picker_alternative",
    ],
    final_pinned_order: await storedPinnedOrder(page),
    browser_console_errors: errors,
    screenshots: [
      path.join(outDir, "project-move-menu.png"),
      path.join(outDir, "pragmatic-dnd-complete.png"),
    ],
  };
  await writeFile(path.join(outDir, "receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.log(JSON.stringify({ ...receipt, output_dir: outDir }));
} catch (error) {
  const receipt = {
    schema_version: 1,
    benchmark: "agent_desktop_pragmatic_dnd_smoke",
    status: "failed",
    mode: "ui_only_no_model_calls",
    browser_console_errors: errors,
    failure: String(error?.stack ?? error),
  };
  await mkdir(outDir, { recursive: true });
  await writeFile(path.join(outDir, "receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.error(JSON.stringify({ ...receipt, output_dir: outDir }));
  process.exitCode = 1;
} finally {
  await browser?.close();
  dev.kill("SIGTERM");
}
