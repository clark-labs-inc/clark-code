import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";

import { launch, VIEWPORT } from "./launch.mjs";

const repoDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
const outDir = process.env.RESILIENCE_E2E_OUTPUT_DIR
  ? path.resolve(process.env.RESILIENCE_E2E_OUTPUT_DIR)
  : path.join(repoDir, "target", "resilience-smoke", `${stamp}-${process.pid}`);
const RESILIENCE_VERSION = 2;
const cases = [
  { id: "clean", mask: 0, expected: "recovered" },
  { id: "new-transport-faults", mask: 38, expected: "recovered" },
  { id: "all-recoverable-faults", mask: 191, expected: "recovered" },
  { id: "provider-process-loss", mask: 64, expected: "paused" },
  { id: "upstream-and-process-loss", mask: 68, expected: "paused" },
  { id: "explicit-user-cancel", mask: 256, expected: "cancelled" },
];

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

const browserErrors = [];
const receipts = [];
let browser;
try {
  await mkdir(outDir, { recursive: true });
  await waitForServer();
  browser = await launch();

  for (const testCase of cases) {
    const context = await browser.newContext({ viewport: VIEWPORT });
    await context.addInitScript(({ mask, version, accountId }) => {
      const accountScope = `id:${accountId}`;
      const encodedScope = encodeURIComponent(accountScope);
      localStorage.setItem("agent-desktop.dev-account", JSON.stringify({
        user: { id: accountId, name: "Resilience QA", method: "local" },
      }));
      localStorage.setItem(`agent-desktop:local-agent:${encodedScope}`, JSON.stringify({
        cwd: "/tmp/resilience-fixture",
        model: "local-model",
        reasoningEffort: "high",
      }));
      localStorage.setItem(`agent-desktop:project-context:${encodedScope}`, JSON.stringify({
        cwd: "/tmp/resilience-fixture",
      }));
      localStorage.setItem("agent-desktop:resilience-benchmark", JSON.stringify({
        version,
        mask,
        delayMs: 5,
      }));
    }, {
      mask: testCase.mask,
      version: RESILIENCE_VERSION,
      accountId: `resilience-${testCase.id}`,
    });
    const page = await context.newPage();
    const caseErrors = [];
    page.on("pageerror", (error) => caseErrors.push(error.stack ?? error.message));
    page.on("console", (message) => {
      if (message.type() === "error") caseErrors.push(message.text());
    });
    await page.goto(url, { waitUntil: "networkidle" });
    const composer = page.getByLabel("Message Clark Code");
    await composer.waitFor({ state: "visible" });
    await composer.fill(`Run resilience case ${testCase.id}`);
    await composer.press("Enter");

    if (testCase.expected === "recovered") {
      await page.getByText("BENCHMARK_OK", { exact: false }).waitFor({ timeout: 10_000 });
      check(await page.getByLabel("Agent paused").count() === 0, `${testCase.id}: recovered run looked paused`);
    } else if (testCase.expected === "paused") {
      const paused = page.getByLabel("Agent paused");
      await paused.waitFor({ state: "visible", timeout: 10_000 });
      await paused.getByRole("button", { name: "Resume task" }).waitFor({ state: "visible" });
      check(await page.getByText("BENCHMARK_OK", { exact: false }).count() === 0, `${testCase.id}: failed run claimed recovery`);
    } else {
      const stop = page.getByRole("button", { name: "Stop", exact: true });
      await stop.waitFor({ state: "visible", timeout: 10_000 });
      await stop.click();
      await stop.waitFor({ state: "hidden", timeout: 10_000 });
      check(await page.getByText("BENCHMARK_OK", { exact: false }).count() === 0, `${testCase.id}: cancelled run claimed recovery`);
    }

    const body = await page.locator("body").innerText();
    for (const internal of ["HTTP 503", "HTTP 504", "tool_execution_host", "provider_route", "benchmark-upstream"]) {
      check(!body.includes(internal), `${testCase.id}: leaked internal diagnostic ${internal}`);
    }
    check(caseErrors.length === 0, `${testCase.id}: browser errors\n${caseErrors.join("\n")}`);
    await page.screenshot({
      path: path.join(outDir, `${testCase.id}.png`),
      animations: "disabled",
      fullPage: true,
    });
    receipts.push({ ...testCase, status: "passed" });
    browserErrors.push(...caseErrors.map((error) => `${testCase.id}: ${error}`));
    await context.close();
  }

  const receipt = {
    schema_version: 1,
    benchmark: "clark_code_hyper_realistic_resilience_smoke",
    status: "passed",
    mode: "mock_provider_no_paid_calls",
    resilience_contract_version: RESILIENCE_VERSION,
    cases: receipts,
    browser_console_errors: browserErrors,
  };
  await writeFile(path.join(outDir, "receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.log(JSON.stringify({ ...receipt, output_dir: outDir }));
} catch (error) {
  const receipt = {
    schema_version: 1,
    benchmark: "clark_code_hyper_realistic_resilience_smoke",
    status: "failed",
    mode: "mock_provider_no_paid_calls",
    resilience_contract_version: RESILIENCE_VERSION,
    cases: receipts,
    browser_console_errors: browserErrors,
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
