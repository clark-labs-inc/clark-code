import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import { launch, VIEWPORT } from "./launch.mjs";

const repoDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
const outDir = process.env.SSH_SETTINGS_OUTPUT_DIR
  ? path.resolve(process.env.SSH_SETTINGS_OUTPUT_DIR)
  : path.join(repoDir, "target", "ssh-settings-smoke", `${stamp}-${process.pid}`);

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
const url = `http://127.0.0.1:${port}/?ssh-config-fixture=1`;
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
const expectedFixtureDenials = [];
const checks = [];
try {
  await mkdir(outDir, { recursive: true });
  await waitForServer();
  browser = await launch();
  const context = await browser.newContext({ viewport: VIEWPORT });
  await context.addInitScript(() => {
    const accountScope = "id:ssh-settings-qa";
    const encodedScope = encodeURIComponent(accountScope);
    localStorage.setItem("agent-desktop.dev-account", JSON.stringify({
      user: { id: "ssh-settings-qa", name: "SSH Settings QA", method: "local" },
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
    if (message.type() !== "error") return;
    const text = message.text();
    if (text === "Failed to load resource: the server responded with a status of 403 (Forbidden)") {
      expectedFixtureDenials.push(text);
      return;
    }
    errors.push(text);
  });
  await page.goto(url, { waitUntil: "domcontentloaded" });

  await page.getByRole("button", { name: /Local/ }).first().click();
  await page.getByRole("button", { name: "Add SSH host…" }).click();
  const dialog = page.getByRole("dialog", { name: "Remote hosts" });
  await dialog.waitFor({ state: "visible" });
  await dialog.getByRole("radio", { name: /gpu-box/ }).click();

  const addButton = dialog.getByRole("button", { name: "Add remote host" });
  check(await addButton.isEnabled(), "a reachable host without a default folder cannot be added");
  checks.push("host_save_without_default_folder");

  const folder = dialog.getByLabel("Remote project folder");
  await folder.fill("/home/ubuntu/project");
  await folder.press("Backspace");
  check(await folder.inputValue() === "/home/ubuntu/projec", "Backspace did not edit the folder path");
  check(await folder.evaluate((element) => document.activeElement === element), "folder focus moved after Backspace");
  checks.push("backspace_preserves_folder_focus");

  await folder.fill("");
  await dialog.getByRole("button", { name: "Test connection" }).click();
  await dialog.getByText("Reachable", { exact: true }).waitFor();
  await page.screenshot({
    path: path.join(outDir, "remote-host-ready-without-folder.png"),
    animations: "disabled",
  });
  await addButton.click();
  await dialog.waitFor({ state: "hidden" });

  const saved = await page.evaluate(() => JSON.parse(
    localStorage.getItem("agent-desktop:ssh-hosts:id%3Assh-settings-qa") ?? "[]",
  ));
  check(
    saved.some((host) => host.host === "gpu-box" && host.remoteRoot === ""),
    "the host was not persisted without a default folder",
  );
  checks.push("host_persisted_without_default_folder");
  const executionTarget = await page.evaluate(() => {
    const store = window.__agentDesktopStore;
    const state = store?.getState();
    return state ? {
      projectMode: state.projectMode,
      selectedHostId: state.selectedHostId,
      selectedHost: JSON.parse(
        localStorage.getItem("agent-desktop:ssh-hosts:id%3Assh-settings-qa") ?? "[]",
      ).find((host) => host.id === state.selectedHostId)?.host ?? null,
    } : null;
  });
  check(
    executionTarget?.projectMode === "remote" && executionTarget.selectedHost === "gpu-box",
    `the saved host did not become the remote execution target: ${JSON.stringify(executionTarget)}`,
  );
  await page.getByRole("button", { name: /gpu-box/ }).first().waitFor();
  await page.getByText("Select remote folder…", { exact: true }).waitFor();
  await page.getByText("Choose a remote folder before starting.", { exact: true }).waitFor();
  check(
    await page.getByText("Remote Git unavailable", { exact: true }).count() === 0,
    "an incomplete host attempted remote Git inspection before a folder was selected",
  );
  await page.screenshot({
    path: path.join(outDir, "saved-host-selected-for-execution.png"),
    animations: "disabled",
  });
  checks.push("saved_host_selected_for_execution");
  check(errors.length === 0, `browser errors:\n${errors.join("\n")}`);

  const receipt = {
    schema_version: 1,
    benchmark: "ssh_settings_smoke",
    status: "passed",
    mode: "ui_fixture_no_live_ssh_or_model_calls",
    viewport: VIEWPORT,
    checks,
    browser_console_errors: errors,
    expected_fixture_network_denials: expectedFixtureDenials.length,
    screenshots: [
      path.join(outDir, "remote-host-ready-without-folder.png"),
      path.join(outDir, "saved-host-selected-for-execution.png"),
    ],
  };
  await writeFile(path.join(outDir, "receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.log(JSON.stringify({ ...receipt, output_dir: outDir }));
} catch (error) {
  const receipt = {
    schema_version: 1,
    benchmark: "ssh_settings_smoke",
    status: "failed",
    mode: "ui_fixture_no_live_ssh_or_model_calls",
    checks,
    browser_console_errors: errors,
    expected_fixture_network_denials: expectedFixtureDenials.length,
    failure: String(error?.stack ?? error),
    body_text: page ? (await page.locator("body").innerText().catch(() => "")).slice(-5000) : "",
  };
  await writeFile(path.join(outDir, "receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.error(JSON.stringify({ ...receipt, output_dir: outDir }));
  process.exitCode = 1;
} finally {
  await browser?.close();
  dev.kill("SIGTERM");
}
