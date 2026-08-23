import { spawn } from "node:child_process";
import { existsSync, statSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";

import { launch } from "./launch.mjs";

const repoDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
const outDir = process.env.MODEL_PICKER_OUTPUT_DIR
  ? path.resolve(process.env.MODEL_PICKER_OUTPUT_DIR)
  : path.join(
      (() => {
        const targetRoot = path.join(repoDir, "target");
        try {
          if (existsSync(targetRoot) && statSync(targetRoot).isDirectory()) return targetRoot;
        } catch {}
        return path.join(tmpdir(), "agent-desktop-harness");
      })(),
      "model-picker-smoke",
      `${stamp}-${process.pid}`,
    );

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

let browser;
let page;
const errors = [];
try {
  await mkdir(outDir, { recursive: true });
  await waitForServer();
  browser = await launch();
  const context = await browser.newContext({ viewport: { width: 261, height: 300 } });
  await context.addInitScript(() => {
    const accountScope = "id:model-picker-qa";
    const encodedScope = encodeURIComponent(accountScope);
    localStorage.setItem("agent-desktop.dev-account", JSON.stringify({
      user: { id: "model-picker-qa", name: "Model Picker QA", method: "local" },
    }));
    localStorage.setItem(`agent-desktop:local-agent:${encodedScope}`, JSON.stringify({
      cwd: "/tmp", model: "local-model", reasoningEffort: "high",
    }));
    localStorage.setItem(
      `agent-desktop:project-context:${encodedScope}`,
      JSON.stringify({ cwd: "/tmp" }),
    );
  });
  page = await context.newPage();
  page.on("pageerror", (error) => errors.push(error.stack ?? error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  await page.goto(url, { waitUntil: "domcontentloaded" });
  await page.getByLabel("Message Clark Code").waitFor({ state: "visible" });

  const picker = page.getByTitle("Model", { exact: true });
  await picker.click();
  const menu = page.getByRole("menu", { name: "Model" });
  await menu.waitFor({ state: "visible" });
  const box = await menu.boundingBox();
  check(Boolean(box), "model menu has no layout box");
  check(box.x >= 0, "model menu is clipped past the left viewport edge");
  check(box.x + box.width <= 261, "model menu is clipped past the right viewport edge");
  check(await menu.evaluate((element) => element.parentElement === document.body), "model menu is not portaled to the document body");
  check(await menu.evaluate((element) => getComputedStyle(element).position === "fixed"), "model menu is not fixed above the workspace");
  await page.screenshot({ path: path.join(outDir, "model-picker-open.png"), animations: "disabled" });

  await page.getByRole("menuitemradio", { name: /Large local model/ }).click();
  await picker.filter({ hasText: "Large local model" }).waitFor();
  await picker.click();
  await page.getByRole("menuitemradio", { name: /^Local model/ }).click();
  await picker.filter({ hasText: /^Local model/ }).waitFor();
  check(errors.length === 0, `browser errors:\n${errors.join("\n")}`);

  const receipt = {
    schema_version: 1,
    benchmark: "agent_desktop_model_picker_smoke",
    status: "passed",
    mode: "ui_only_no_model_calls",
    viewport: { width: 261, height: 300 },
    checks: ["body_portal", "compact_bounds", "pointer_select_large", "pointer_select_default"],
    browser_console_errors: errors,
    screenshot: path.join(outDir, "model-picker-open.png"),
  };
  await writeFile(path.join(outDir, "receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.log(JSON.stringify({ ...receipt, output_dir: outDir }));
} catch (error) {
  const receipt = {
    schema_version: 1,
    benchmark: "agent_desktop_model_picker_smoke",
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
