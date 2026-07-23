import { spawn } from "node:child_process";
import { createConnection } from "node:net";
import { mkdtemp, mkdir, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import { launch, VIEWPORT } from "./launch.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const appDir = path.join(repoDir, "app");
const contract = JSON.parse(await readFile(
  path.join(appDir, "src/core-bridge/resilienceBenchmark.json"),
  "utf8",
));
const appUrl = "http://localhost:1420";
const args = new Set(process.argv.slice(2));
const liveOnly = args.has("--live-only");
const includeLive = liveOnly || args.has("--live");
const smokeOnly = args.has("--smoke");
const selectedCase = process.argv.slice(2).find((value) => value.startsWith("--case="));
const requestedMask = selectedCase ? Number(selectedCase.slice("--case=".length)) : null;
const selectedOutput = process.argv.slice(2).find((value) => value.startsWith("--out="));
const requestedOutput = selectedOutput ? selectedOutput.slice("--out=".length) : null;

if (requestedMask !== null && (!Number.isInteger(requestedMask) || requestedMask < 0 || requestedMask >= 2 ** contract.faults.length)) {
  throw new Error(`--case must be an integer from 0 to ${(2 ** contract.faults.length) - 1}`);
}
if (smokeOnly && requestedMask !== null) {
  throw new Error("--smoke and --case are mutually exclusive");
}
if (requestedOutput === "") {
  throw new Error("--out requires a directory");
}

const artifactDir = requestedOutput
  ? path.resolve(process.cwd(), requestedOutput)
  : await mkdtemp(path.join(tmpdir(), "clark-desktop-resilience-"));
if (requestedOutput) {
  await mkdir(path.dirname(artifactDir), { recursive: true });
  await mkdir(artifactDir);
}
await mkdir(path.join(artifactDir, "simulated"), { recursive: true });
await mkdir(path.join(artifactDir, "live"), { recursive: true });

const children = [];
let browser;

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function waitForExit(child, timeoutMs) {
  if (child.exitCode !== null) return Promise.resolve(true);
  return new Promise((resolve) => {
    const timeout = setTimeout(() => resolve(false), timeoutMs);
    child.once("exit", () => {
      clearTimeout(timeout);
      resolve(true);
    });
  });
}

function signalChildTree(child, signal) {
  if (child.exitCode !== null) return;
  if (process.platform !== "win32" && child.pid) {
    try {
      process.kill(-child.pid, signal);
      return;
    } catch {
      // Fall back to the direct child if its process group has already exited.
    }
  }
  child.kill(signal);
}

async function stopChildTree(child) {
  if (child.exitCode !== null) return;
  signalChildTree(child, "SIGTERM");
  if (await waitForExit(child, 5_000)) return;
  signalChildTree(child, "SIGKILL");
  await waitForExit(child, 5_000);
}

async function urlReady(url) {
  try {
    const response = await fetch(url);
    return response.ok;
  } catch {
    return false;
  }
}

async function waitForUrl(url, child, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await urlReady(url)) return;
    if (child?.exitCode !== null && child?.exitCode !== undefined) {
      throw new Error(`server exited early with code ${child.exitCode}`);
    }
    await sleep(250);
  }
  throw new Error(`timed out waiting for ${url}`);
}

async function ensureVite() {
  if (await urlReady(appUrl)) return null;
  const child = spawn("pnpm", ["dev"], {
    cwd: appDir,
    detached: process.platform !== "win32",
    stdio: "ignore",
    env: { ...process.env },
  });
  children.push(child);
  await waitForUrl(appUrl, child);
  return child;
}

function waitForPort(port, timeoutMs = 180_000) {
  const deadline = Date.now() + timeoutMs;
  return new Promise((resolve, reject) => {
    const probe = () => {
      const socket = createConnection({ host: "127.0.0.1", port });
      socket.once("connect", () => {
        socket.destroy();
        resolve();
      });
      socket.once("error", () => {
        socket.destroy();
        if (Date.now() >= deadline) reject(new Error(`timed out waiting for port ${port}`));
        else setTimeout(probe, 300);
      });
    };
    probe();
  });
}

function faultsForMask(mask) {
  return contract.faults.filter((_, index) => (mask & (1 << index)) !== 0);
}

function expectedIncidentCount(faults) {
  return [
    "rate_limit",
    "event_stream_disconnect",
    "provider_process_loss",
    "cloud_sync_delay",
  ].filter((fault) => faults.includes(fault)).length;
}

function browserSeed({
  mask,
  model,
  storageKey,
  version,
  apiKey = "benchmark-not-a-real-key",
}) {
  localStorage.clear();
  localStorage.setItem("clark.auth.session", JSON.stringify({
    user: {
      id: "resilience-benchmark",
      name: "Resilience Benchmark",
      email: "resilience-benchmark@clark.local",
      method: "local",
    },
    clark: { endpoint: "wss://api.clarkslabs.com/ws", token: "benchmark-session" },
  }));
  localStorage.setItem("clark-desktop:local-agent", JSON.stringify({
    cwd: "/tmp/clark-desktop-resilience-fixture",
    model,
    reasoningEffort: "",
    apiKey,
  }));
  if (mask === null) {
    localStorage.removeItem(storageKey);
  } else {
    localStorage.setItem(storageKey, JSON.stringify({
      version,
      mask,
      delayMs: 180,
    }));
  }
}

async function waitForTerminal(page, faults) {
  if (faults.includes("user_cancel")) {
    await page.getByText("Run stopped before finishing.").waitFor({ timeout: 8_000 });
  } else if (faults.includes("provider_process_loss")) {
    await page.getByText("Run interrupted.").waitFor({ timeout: 8_000 });
  } else {
    await page.getByText("BENCHMARK_OK").waitFor({ timeout: 8_000 });
  }
  await page.waitForFunction(() => Array.from(
    document.querySelectorAll('section[aria-label="Provider incident"]'),
  ).every((card) => !card.textContent?.includes("Retrying")), null, { timeout: 8_000 });
}

async function assertHealthySurface(page, expectedIncidents) {
  const body = await page.locator("body").innerText();
  const normalized = body.replace(/\s+/g, " ").trim();
  if (normalized.length < 120) throw new Error("conversation surface rendered too little content");
  for (const forbidden of [
    "Panel failed to render",
    "Cannot read properties of undefined",
    "shell:89",
    "clark_agent_call_1",
  ]) {
    if (body.includes(forbidden)) throw new Error(`visible implementation detail: ${forbidden}`);
  }
  const incidents = await page.locator('section[aria-label="Provider incident"]').count();
  if (incidents !== expectedIncidents) {
    throw new Error(`expected ${expectedIncidents} provider incident cards, found ${incidents}`);
  }
  const main = page.locator("main").first();
  if (await main.count()) {
    const box = await main.boundingBox();
    if (!box || box.width < 300 || box.height < 300) throw new Error("main conversation surface is blank");
  }
}

async function continueAfterTerminal(page) {
  await page.evaluate((storageKey) => {
    localStorage.setItem(storageKey, JSON.stringify({ version: 1, mask: 0, delayMs: 20 }));
  }, contract.storageKey);
  const resume = page.getByRole("button", { name: "Continue from saved progress" });
  if (await resume.count()) {
    await resume.last().click();
  } else {
    const composer = page.getByRole("textbox", { name: "Message Clark" });
    await composer.fill("Continue from the saved progress and verify this conversation is still usable.");
    await composer.press("Enter");
  }
  await page.getByText("BENCHMARK_OK").last().waitFor({ timeout: 8_000 });
}

async function runSimulatedCase(mask) {
  const faults = faultsForMask(mask);
  const id = mask.toString(2).padStart(contract.faults.length, "0");
  const context = await browser.newContext({ viewport: VIEWPORT, reducedMotion: "reduce" });
  await context.addInitScript(browserSeed, {
    mask,
    model: contract.model,
    storageKey: contract.storageKey,
    version: contract.version,
  });
  const page = await context.newPage();
  const consoleErrors = [];
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(message.text());
  });
  page.on("pageerror", (error) => consoleErrors.push(error.message));

  const started = Date.now();
  try {
    await page.goto(appUrl, { waitUntil: "domcontentloaded" });
    const composer = page.getByRole("textbox", { name: "Message Clark" });
    await composer.waitFor({ timeout: 10_000 });
    await page.getByText(contract.modelLabel, { exact: true }).waitFor({ timeout: 10_000 });
    await composer.fill(`Resilience benchmark case ${id}. Verify recovery without changing project files.`);
    await composer.press("Enter");

    const incidentCount = expectedIncidentCount(faults);
    if (incidentCount > 0) {
      await page.locator('section[aria-label="Provider incident"]').nth(incidentCount - 1)
        .waitFor({ timeout: 8_000 });
      await page.screenshot({
        path: path.join(artifactDir, "simulated", `${id}-active.png`),
        fullPage: true,
      });
    }
    if (faults.includes("user_cancel")) {
      const stop = page.getByRole("button", { name: "Stop" });
      await stop.waitFor({ timeout: 8_000 });
      await stop.click();
    }

    await waitForTerminal(page, faults);
    await assertHealthySurface(page, incidentCount);
    await page.screenshot({
      path: path.join(artifactDir, "simulated", `${id}-terminal.png`),
      fullPage: true,
    });

    if (faults.includes("user_cancel") || faults.includes("provider_process_loss")) {
      await continueAfterTerminal(page);
      await assertHealthySurface(page, incidentCount);
    }
    if (consoleErrors.length > 0) {
      throw new Error(`browser errors: ${consoleErrors.slice(0, 3).join(" | ")}`);
    }
    return {
      id,
      mask,
      kind: "simulated",
      model: contract.model,
      provider: contract.provider,
      faults,
      status: "passed",
      durationMs: Date.now() - started,
      terminalScreenshot: path.join(artifactDir, "simulated", `${id}-terminal.png`),
    };
  } catch (error) {
    await page.screenshot({
      path: path.join(artifactDir, "simulated", `${id}-failure.png`),
      fullPage: true,
    }).catch(() => {});
    return {
      id,
      mask,
      kind: "simulated",
      model: contract.model,
      provider: contract.provider,
      faults,
      status: "failed",
      durationMs: Date.now() - started,
      error: error instanceof Error ? error.message : String(error),
    };
  } finally {
    await context.close();
  }
}

function parseDotEnv(name) {
  return readFile(path.join(repoDir, ".env"), "utf8")
    .then((text) => {
      for (const line of text.split(/\r?\n/)) {
        const match = line.match(/^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)\s*$/);
        if (!match || match[1] !== name) continue;
        const value = match[2].replace(/^(['"])(.*)\1$/, "$2").trim();
        return value || null;
      }
      return null;
    })
    .catch(() => null);
}

async function runLiveControl() {
  const apiKey = process.env.CLARK_CODE_API_KEY || await parseDotEnv("CLARK_CODE_API_KEY");
  if (!apiKey) throw new Error("CLARK_CODE_API_KEY is required for --live");
  const fixture = await mkdtemp(path.join(tmpdir(), "clark-desktop-deepseek-"));
  await writeFile(
    path.join(fixture, "README.md"),
    "# Clark Desktop DeepSeek control\n\nThe verification token is LIVE_DEEPSEEK_OK.\n",
  );

  const devbridge = spawn("cargo", ["run", "-p", "devbridge"], {
    cwd: repoDir,
    detached: process.platform !== "win32",
    stdio: "ignore",
    env: {
      ...process.env,
      DEVBRIDGE_ADDR: "127.0.0.1:7878",
      RUST_LOG: "devbridge=info,provider_local=info",
    },
  });
  children.push(devbridge);
  await waitForPort(7878);

  const context = await browser.newContext({ viewport: VIEWPORT, reducedMotion: "reduce" });
  await context.addInitScript(({ key, cwd, model, storageKey }) => {
    localStorage.clear();
    localStorage.setItem("clark.auth.session", JSON.stringify({
      user: {
        id: "resilience-benchmark-live",
        name: "Resilience Benchmark",
        email: "resilience-benchmark@clark.local",
        method: "local",
      },
      clark: { endpoint: "wss://api.clarkslabs.com/ws", token: "benchmark-session" },
    }));
    localStorage.setItem("clark-desktop:local-agent", JSON.stringify({
      cwd,
      model,
      reasoningEffort: "",
      apiKey: key,
    }));
    localStorage.removeItem(storageKey);
  }, {
    key: apiKey,
    cwd: fixture,
    model: contract.model,
    storageKey: contract.storageKey,
  });
  const page = await context.newPage();
  const consoleErrors = [];
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(message.text());
  });
  page.on("pageerror", (error) => consoleErrors.push(error.message));
  const started = Date.now();
  try {
    await page.goto(`${appUrl}/?dev=ws://127.0.0.1:7878`, { waitUntil: "domcontentloaded" });
    const composer = page.getByRole("textbox", { name: "Message Clark" });
    await composer.waitFor({ timeout: 30_000 });
    await page.getByText(contract.modelLabel, { exact: true }).waitFor({ timeout: 30_000 });
    await composer.fill(
      "Read README.md. Then reply with LIVE_DEEPSEEK_OK and one short sentence confirming the file was read. Do not edit any files.",
    );
    await composer.press("Enter");
    await page.getByRole("button", { name: "Stop" }).waitFor({ timeout: 30_000 });
    await page.screenshot({ path: path.join(artifactDir, "live", "deepseek-active.png"), fullPage: true });
    const assistantReply = page
      .locator('button[aria-label="Copy as Markdown"]')
      .locator("..")
      .filter({ hasText: "LIVE_DEEPSEEK_OK" });
    await assistantReply.last().waitFor({ timeout: 300_000 });
    await page.getByRole("button", { name: "Stop" }).waitFor({ state: "hidden", timeout: 30_000 });
    await assertHealthySurface(page, 0);
    await page.screenshot({ path: path.join(artifactDir, "live", "deepseek-final.png"), fullPage: true });
    if (consoleErrors.length > 0) {
      throw new Error(`browser errors: ${consoleErrors.slice(0, 3).join(" | ")}`);
    }
    return {
      id: "deepseek-live-control",
      kind: "live",
      model: contract.model,
      provider: contract.provider,
      status: "passed",
      durationMs: Date.now() - started,
      finalScreenshot: path.join(artifactDir, "live", "deepseek-final.png"),
    };
  } finally {
    await context.close();
    await stopChildTree(devbridge);
  }
}

const report = {
  contract,
  artifactDir,
  selection: liveOnly ? "live_only" : requestedMask !== null ? "single_case" : smokeOnly ? "smoke" : "full",
  startedAt: new Date().toISOString(),
  simulated: [],
  live: null,
};

try {
  await ensureVite();
  browser = await launch();
  if (!liveOnly) {
    const allFaultsMask = (2 ** contract.faults.length) - 1;
    const smokeMasks = [0, ...contract.faults.map((_, index) => 1 << index), allFaultsMask];
    const masks = requestedMask !== null
      ? [requestedMask]
      : smokeOnly
        ? [...new Set(smokeMasks)]
        : Array.from({ length: 2 ** contract.faults.length }, (_, mask) => mask);
    for (const mask of masks) {
      const result = await runSimulatedCase(mask);
      report.simulated.push(result);
      process.stdout.write(result.status === "passed" ? "." : "F");
    }
    process.stdout.write("\n");
  }
  if (includeLive) report.live = await runLiveControl();
} finally {
  report.finishedAt = new Date().toISOString();
  report.summary = {
    simulatedPassed: report.simulated.filter((value) => value.status === "passed").length,
    simulatedFailed: report.simulated.filter((value) => value.status === "failed").length,
    livePassed: report.live?.status === "passed" ? 1 : 0,
  };
  await writeFile(path.join(artifactDir, "report.json"), `${JSON.stringify(report, null, 2)}\n`);
  if (browser) await browser.close();
  for (const child of children) {
    await stopChildTree(child);
  }
}

console.log(JSON.stringify(report.summary));
console.log(`REPORT=${path.join(artifactDir, "report.json")}`);
const failed = report.simulated.filter((value) => value.status === "failed");
if (failed.length > 0) {
  console.error(JSON.stringify(failed, null, 2));
  process.exitCode = 1;
}
