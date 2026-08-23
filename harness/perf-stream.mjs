// Scenario A1: a deterministic streaming run through the real UI, recorded.
//
// Drives the store at a fixed cadence with a transcript that GROWS over the
// run, because the costs under investigation scale with transcript length — a
// fixed-size replay measures the one case that does not hurt.
//
//   node harness/perf-stream.mjs                     # webkit (engine fidelity)
//   ENGINE=chromium node harness/perf-stream.mjs     # + CDP CPU attribution
//   TURNS=60 CADENCE_MS=16 node harness/perf-stream.mjs
//   PERF_ALLOW_NOISY=1 node harness/perf-stream.mjs  # validate, do not believe
//
// Playwright's webkit is a patched WebKit build, not the platform WebView, so
// this is a proxy engine: good for attribution and for regression comparison,
// not a substitute for measuring the packaged app. The manifest records that.
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { mkdirSync, writeFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { execFileSync } from "node:child_process";
import { chromium, webkit } from "playwright";

import { preflight } from "./perf-preflight.mjs";

const root = new URL("..", import.meta.url).pathname;
const engineName = process.env.ENGINE ?? "webkit";
const turns = Number(process.env.TURNS ?? 24);
const cadenceMs = Number(process.env.CADENCE_MS ?? 16);
const codeLines = Number(process.env.CODE_LINES ?? 60);
const chunksPerTurn = Number(process.env.CHUNKS_PER_TURN ?? 25);
const allowNoisy = process.env.PERF_ALLOW_NOISY === "1";

function reservePort() {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close((error) => (error ? reject(error) : resolve(port)));
    });
  });
}

const seedLocalStorage = `
  const scope = encodeURIComponent("id:perf-qa");
  localStorage.setItem("agent-desktop.dev-account", JSON.stringify({
    user: { id: "perf-qa", name: "Perf QA", method: "local" },
  }));
  localStorage.setItem('agent-desktop:local-agent:' + scope, JSON.stringify({
    cwd: "/tmp", model: "local-model", reasoningEffort: "high",
  }));
  localStorage.setItem('agent-desktop:project-context:' + scope, JSON.stringify({ cwd: "/tmp" }));
`;

/** A compact, sortable UTC stamp: 20260822T184530Z. */
function utcStamp(date = new Date()) {
  return `${date.toISOString().replace(/[-:]/g, "").replace(/\.\d+Z$/, "")}Z`;
}

function gitSha() {
  try {
    return execFileSync("git", ["rev-parse", "--short", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
  } catch {
    return "unknown";
  }
}

const before = preflight();
if (!before.quiet && !allowNoisy) {
  console.error("Machine is not quiet enough to measure:");
  for (const failure of before.failures) console.error(`  - ${failure}`);
  console.error("\nRe-run with PERF_ALLOW_NOISY=1 to exercise the recorder anyway.");
  console.error("Numbers from a noisy run describe the machine, not the app.");
  process.exit(1);
}
if (!before.quiet) {
  console.warn("WARNING: machine is not quiet. This run validates the recorder; do not");
  console.warn("         quote its numbers as a baseline.\n");
}

const port = await reservePort();
const devUrl = `http://127.0.0.1:${port}/`;
const vite = spawn(
  "node",
  ["node_modules/vite/bin/vite.js", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
  {
    cwd: `${root}app`,
    // VITE_PERF_HOOKS installs the recorder; VITE_PRODUCT_DEV_AUTH lets the
    // seeded local account through without credentials.
    env: { ...process.env, VITE_PERF_HOOKS: "1", VITE_PRODUCT_DEV_AUTH: "1" },
    stdio: ["ignore", "pipe", "pipe"],
  },
);
let viteLog = "";
vite.stdout.on("data", (chunk) => (viteLog += chunk));
vite.stderr.on("data", (chunk) => (viteLog += chunk));

async function waitForDevServer() {
  for (let attempt = 0; attempt < 200; attempt += 1) {
    if (vite.exitCode !== null) throw new Error(`vite exited:\n${viteLog}`);
    try {
      if ((await fetch(devUrl)).ok) return;
    } catch { /* not up yet */ }
    await sleep(150);
  }
  throw new Error(`vite did not start:\n${viteLog}`);
}

const runId = `${utcStamp()}-${gitSha()}-streamA1-${engineName}`;
const outDir = `${root}target/perf/${runId}`;
mkdirSync(outDir, { recursive: true });

let browser;
const consoleErrors = [];
try {
  await waitForDevServer();
  const engine = engineName === "chromium" ? chromium : webkit;
  browser = await engine.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 1360, height: 880 } });
  await context.addInitScript(seedLocalStorage);
  const page = await context.newPage();
  page.on("pageerror", (error) => consoleErrors.push(`PAGEERROR ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(message.text());
  });

  await page.goto(devUrl, { waitUntil: "domcontentloaded" });
  await page.getByLabel("Message Clark Code").waitFor({ state: "visible", timeout: 60_000 });
  await page.waitForFunction(() => "__clarkPerf" in window, null, { timeout: 30_000 });

  // The cadence this window actually got. A budget derived from an assumed
  // 60 Hz is meaningless if the engine is running at some other rate.
  const baselinePeriodMs = await page.evaluate(
    () => window.__clarkPerf.measureBaselinePeriod(2000),
  );
  console.log(`observed frame period: ${baselinePeriodMs.toFixed(2)} ms`
    + ` (${(1000 / baselinePeriodMs).toFixed(0)} Hz)`);

  const capabilities = await page.evaluate(() => window.__clarkPerf.capabilities);
  console.log(`clock resolution: ${capabilities.clockResolutionMs} ms`);
  console.log(`substituted metrics: ${capabilities.missingEntryTypes.join(", ") || "none"}`);

  let cdp = null;
  if (engineName === "chromium") {
    cdp = await context.newCDPSession(page);
    await cdp.send("Profiler.enable");
    await cdp.send("Profiler.setSamplingInterval", { interval: 100 });
    await cdp.send("Profiler.start");
  }

  await page.evaluate(() => window.__clarkPerf.start("streamA1"));
  const replay = await page.evaluate(
    (options) => window.__clarkPerf.replayStream(options),
    { turns, cadenceMs, codeLines, chunksPerTurn },
  );
  const summary = await page.evaluate(() => window.__clarkPerf.stop());

  if (cdp) {
    const { profile } = await cdp.send("Profiler.stop");
    writeFileSync(`${outDir}/cpu.cpuprofile`, JSON.stringify(profile));
    // Self-time leaderboard, same aggregation as profile-chat-switch.mjs.
    const byId = new Map(profile.nodes.map((node) => [node.id, node]));
    const selfUs = new Map();
    const deltas = profile.timeDeltas ?? [];
    (profile.samples ?? []).forEach((id, index) => {
      selfUs.set(id, (selfUs.get(id) ?? 0) + (deltas[index] ?? 0));
    });
    const leaderboard = [...selfUs.entries()]
      .map(([id, us]) => {
        const frame = byId.get(id)?.callFrame;
        if (!frame) return null;
        const file = frame.url.split("/").slice(-2).join("/");
        return { fn: `${frame.functionName || "(anon)"} @ ${file}:${frame.lineNumber + 1}`, selfMs: us / 1000 };
      })
      .filter(Boolean)
      .sort((a, b) => b.selfMs - a.selfMs)
      .slice(0, 25);
    writeFileSync(`${outDir}/top-functions.json`, JSON.stringify(leaderboard, null, 2));
    console.log("\nTOP SELF-TIME (main thread):");
    for (const row of leaderboard.slice(0, 12)) {
      console.log(`  ${row.selfMs.toFixed(1).padStart(8)}ms  ${row.fn}`);
    }
  }

  const after = preflight();
  const manifest = {
    runId,
    scenario: "streamA1",
    engine: engineName,
    // A proxy engine over the dev server: React is in development mode and
    // StrictMode double-renders, so absolute numbers are an upper bound.
    fidelity: "playwright-proxy-engine + vite dev + StrictMode (upper bound)",
    perturbed: engineName === "chromium" ? "cdp-profiler" : "none",
    gitSha: gitSha(),
    replay,
    baselinePeriodMs,
    preflightBefore: before,
    preflightAfter: after,
    quiet: before.quiet && after.quiet,
    consoleErrors,
  };
  writeFileSync(`${outDir}/manifest.json`, JSON.stringify(manifest, null, 2));
  writeFileSync(
    `${outDir}/summary.json`,
    JSON.stringify({ ...summary, perturbed: manifest.perturbed }, null, 2),
  );

  console.log(`\nreplayed ${replay.pushes} snapshot pushes over ${replay.turns} turns`
    + ` in ${(replay.elapsedMs / 1000).toFixed(1)}s`);
  console.log(`dropped frames: ${summary.frameLoss.droppedFrames}`
    + ` (${(summary.frameLoss.droppedRatio * 100).toFixed(1)}%),`
    + ` longest gap ${summary.frameLoss.longestGapPeriods} periods`);
  for (const [name, metric] of Object.entries(summary.metrics)) {
    if (metric.n === 0) continue;
    const budget = metric.budget === undefined ? "" : `  budget ${metric.budget} ${metric.pass ? "PASS" : "FAIL"}`;
    console.log(`  ${name.padEnd(22)} p50 ${metric.p50.toFixed(2).padStart(9)}`
      + `  p95 ${metric.p95.toFixed(2).padStart(9)}`
      + `  max ${metric.max.toFixed(2).padStart(9)} ${metric.unit}${budget}`);
  }
  console.log("\ngrowth per timeline item:");
  for (const [name, slope] of Object.entries(summary.growth)) {
    console.log(`  ${name.padEnd(42)} ${slope.toFixed(4)}`);
  }
  console.log(`\nrootMutations during run: ${summary.rootMutations}`);
  if (consoleErrors.length > 0) {
    console.log(`\npage errors (${consoleErrors.length}):`);
    for (const error of consoleErrors.slice(0, 5)) console.log(`  ${error}`);
  }
  console.log(`\nartifacts: ${outDir}`);
} finally {
  await browser?.close();
  vite.kill("SIGTERM");
}
