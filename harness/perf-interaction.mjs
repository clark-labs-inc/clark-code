// Scenario B: idle interactions, and what a sidebar update actually costs.
//
// The streaming scenario says nothing about clicks, menus, and hovers — the
// other half of the reported jitter. This seeds a realistic sidebar, then
// measures three things:
//   1. a pure-idle window, to prove the app does nothing when nothing happens
//   2. the cost of one store update that only affects one sidebar row
//   3. click-to-paint latency on a sidebar row
//
//   node harness/perf-interaction.mjs
//   CONVERSATIONS=200 node harness/perf-interaction.mjs
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { mkdirSync, writeFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { execFileSync } from "node:child_process";
import { chromium } from "playwright";

import { preflight } from "./perf-preflight.mjs";

const root = new URL("..", import.meta.url).pathname;
const conversationCount = Number(process.env.CONVERSATIONS ?? 120);
const idleMs = Number(process.env.IDLE_MS ?? 6000);
const allowNoisy = process.env.PERF_ALLOW_NOISY === "1";

/** A compact, sortable UTC stamp: 20260822T184530Z. */
function utcStamp(date = new Date()) {
  return `${date.toISOString().replace(/[-:]/g, "").replace(/\.\d+Z$/, "")}Z`;
}

const before = preflight();
if (!before.quiet && !allowNoisy) {
  console.error("Machine is not quiet:");
  for (const f of before.failures) console.error(`  - ${f}`);
  console.error("\nRe-run with PERF_ALLOW_NOISY=1 to exercise the harness anyway.");
  process.exit(1);
}
if (!before.quiet) console.warn("WARNING: noisy machine — relative numbers only.\n");

const port = await new Promise((res, rej) => {
  const s = createServer();
  s.once("error", rej);
  s.listen(0, "127.0.0.1", () => { const { port } = s.address(); s.close(() => res(port)); });
});
const url = `http://127.0.0.1:${port}/`;
const vite = spawn("node", ["node_modules/vite/bin/vite.js", "--host", "127.0.0.1", "--port", String(port), "--strictPort"], {
  cwd: `${root}app`,
  env: { ...process.env, VITE_PERF_HOOKS: "1", VITE_PRODUCT_DEV_AUTH: "1" },
  stdio: ["ignore", "pipe", "pipe"],
});
let log = "";
vite.stdout.on("data", (c) => (log += c));
vite.stderr.on("data", (c) => (log += c));

const seed = `
  const scope = encodeURIComponent("id:perf-qa");
  localStorage.setItem("agent-desktop.dev-account", JSON.stringify({ user: { id: "perf-qa", name: "Perf QA", method: "local" } }));
  localStorage.setItem('agent-desktop:local-agent:' + scope, JSON.stringify({ cwd: "/tmp", model: "local-model", reasoningEffort: "high" }));
  localStorage.setItem('agent-desktop:project-context:' + scope, JSON.stringify({ cwd: "/tmp" }));
`;

let browser;
try {
  for (let i = 0; i < 200; i += 1) {
    if (vite.exitCode !== null) throw new Error(`vite exited:\n${log}`);
    try { if ((await fetch(url)).ok) break; } catch {}
    await sleep(150);
  }
  browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 1360, height: 880 } });
  await context.addInitScript(seed);
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e.message)));
  page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });

  await page.goto(url, { waitUntil: "domcontentloaded" });
  await page.getByLabel("Message Clark Code").waitFor({ state: "visible", timeout: 60_000 });
  await page.waitForFunction(() => "__clarkPerf" in window, null, { timeout: 30_000 });

  // A realistic sidebar. Every row is a layout-animated Motion node inside a
  // nested AnimatePresence, so the count is the thing that matters.
  await page.evaluate((count) => {
    const now = Date.now();
    const conversations = Array.from({ length: count }, (_, i) => ({
      id: `conv-${i}`,
      title: `Conversation number ${i} about some engineering topic`,
      provider: "local",
      project: `/tmp/project-${i % 6}`,
      createdAt: now - i * 60_000,
      updatedAt: now - i * 60_000,
    }));
    window.__agentDesktopStore.setState({ conversations, conversationsLoading: false });
  }, conversationCount);
  await sleep(800);

  const domSize = await page.evaluate(() => ({
    total: document.querySelectorAll("#root *").length,
    rows: document.querySelectorAll("[data-sidebar-conversation-id]").length,
  }));
  console.log(`sidebar: ${domSize.rows} rows, ${domSize.total} elements under #root`);

  // 1. Idle window: any mutation or block here is the app working when it
  //    should be still, which is the "jitter while doing nothing" complaint.
  const idle = await page.evaluate(async (ms) => {
    await window.__clarkPerf.start("idle");
    await new Promise((r) => setTimeout(r, ms));
    return window.__clarkPerf.stop({ droppedRatio: 0, blockP99Ms: 16, blockMaxMs: 16 });
  }, idleMs);
  console.log(`\nidle ${idleMs}ms: rootMutations=${idle.rootMutations}`
    + ` droppedFrames=${idle.frameLoss.droppedFrames}`
    + ` blockP95=${idle.metrics.blockMs.p95.toFixed(1)}ms`
    + ` blockMax=${idle.metrics.blockMs.max.toFixed(1)}ms`);

  // 2. One store update that changes a single row's `streaming` flag. If the
  //    sidebar re-renders every row for this, that is the cost to remove.
  const rowUpdate = await page.evaluate(async () => {
    const store = window.__agentDesktopStore;
    // Control: the same wait with a state write that changes nothing. Two
    // animation frames cannot elapse in less than two frame periods, so this
    // is the floor of the measurement — the real work is the excess over it.
    const control = [];
    for (let i = 0; i < 12; i += 1) {
      await new Promise((resolve) => {
        requestAnimationFrame(() => {
          const t0 = performance.now();
          store.setState({});
          requestAnimationFrame(() => requestAnimationFrame(() => {
            control.push(performance.now() - t0);
            resolve();
          }));
        });
      });
    }
    control.sort((a, b) => a - b);
    const samples = [];
    for (let i = 0; i < 12; i += 1) {
      const id = `conv-${i}`;
      await new Promise((resolve) => {
        requestAnimationFrame(() => {
          const t0 = performance.now();
          store.setState({ runningIds: [id] });
          requestAnimationFrame(() => requestAnimationFrame(() => {
            samples.push(performance.now() - t0);
            resolve();
          }));
        });
      });
    }
    samples.sort((a, b) => a - b);
    return {
      median: samples[Math.floor(samples.length / 2)],
      max: samples[samples.length - 1],
      controlMedian: control[Math.floor(control.length / 2)],
    };
  });
  console.log(`one-row store update -> painted: median ${rowUpdate.median.toFixed(1)}ms,`
    + ` max ${rowUpdate.max.toFixed(1)}ms`);
  console.log(`  measurement floor (empty state write): ${rowUpdate.controlMedian.toFixed(1)}ms`
    + `  -> work above floor: ${(rowUpdate.median - rowUpdate.controlMedian).toFixed(1)}ms`);

  // 3. A real click on a sidebar row, measured to the frame after it lands.
  const cdp = await context.newCDPSession(page);
  await cdp.send("Profiler.enable");
  await cdp.send("Profiler.setSamplingInterval", { interval: 100 });
  await cdp.send("Profiler.start");
  const clickSamples = [];
  for (let i = 0; i < 8; i += 1) {
    const row = page.locator(`[data-sidebar-conversation-id="conv-${i}"] button`).first();
    const t0 = Date.now();
    await row.click({ timeout: 5000 }).catch(() => {});
    await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
    clickSamples.push(Date.now() - t0);
  }
  const { profile } = await cdp.send("Profiler.stop");
  await cdp.detach();
  clickSamples.sort((a, b) => a - b);
  console.log(`row click -> painted: median ${clickSamples[Math.floor(clickSamples.length / 2)]}ms,`
    + ` max ${clickSamples[clickSamples.length - 1]}ms`);

  const byId = new Map(profile.nodes.map((n) => [n.id, n]));
  const selfUs = new Map();
  const deltas = profile.timeDeltas ?? [];
  (profile.samples ?? []).forEach((id, i) => selfUs.set(id, (selfUs.get(id) ?? 0) + (deltas[i] ?? 0)));
  const top = [...selfUs.entries()]
    .map(([id, us]) => {
      const f = byId.get(id)?.callFrame;
      if (!f) return null;
      return { fn: `${f.functionName || "(anon)"} @ ${f.url.split("/").slice(-2).join("/")}:${f.lineNumber + 1}`, selfMs: us / 1000 };
    })
    .filter(Boolean)
    .sort((a, b) => b.selfMs - a.selfMs)
    .slice(0, 15);
  console.log("\nTOP SELF-TIME during row clicks:");
  for (const r of top) console.log(`  ${r.selfMs.toFixed(1).padStart(8)}ms  ${r.fn.slice(0, 100)}`);

  const sha = execFileSync("git", ["rev-parse", "--short", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
  const runId = `${utcStamp()}-${sha}-interactionB-chromium`;
  const outDir = `${root}target/perf/${runId}`;
  mkdirSync(outDir, { recursive: true });
  writeFileSync(
    `${outDir}/summary.json`,
    JSON.stringify({ ...idle, scenario: "interactionB-idle", perturbed: "cdp-profiler" }, null, 2),
  );
  writeFileSync(`${outDir}/interaction.json`, JSON.stringify({ domSize, rowUpdate, clickSamples, top }, null, 2));
  writeFileSync(`${outDir}/manifest.json`, JSON.stringify({
    runId, scenario: "interactionB", conversationCount, engine: "chromium",
    fidelity: "playwright chromium + vite dev + StrictMode (upper bound)",
    perturbed: "cdp-profiler", preflightBefore: before, preflightAfter: preflight(), errors,
  }, null, 2));
  if (errors.length) console.log(`\npage errors: ${errors.slice(0, 3).join(" | ")}`);
  console.log(`\nartifacts: ${outDir}`);
} finally {
  await browser?.close();
  vite.kill("SIGTERM");
}
