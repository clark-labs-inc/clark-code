// Profile chat-to-chat switching: seed two heavy mock conversations through the
// REAL app (MockBridge, no live model), then switch between the two sidebar
// chats while capturing a CDP CPU profile. Answers: how long a warm reattach
// takes end-to-end, and which functions burn the main thread during it.
//
// Runs against the Vite DEV server (React dev mode — numbers include the ~2×
// StrictMode/dev overhead, so treat them as an UPPER BOUND; the shipped Tauri
// app runs the prod bundle and is faster. Relative attribution still holds).
//
// Run:  node harness/profile-chat-switch.mjs
// Env:  PORT (default 1421), ROUNDS (alternations, default 4), TURNS (heavy
//       turns per conversation, default 2).
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const root = fileURLToPath(new URL("..", import.meta.url));
const port = Number(process.env.PORT ?? 1421);
const rounds = Number(process.env.ROUNDS ?? 4);
const turns = Number(process.env.TURNS ?? 2);
const url = `http://127.0.0.1:${port}/`;

const appDir = fileURLToPath(new URL("../app", import.meta.url));
const dev = spawn(
  process.execPath,
  ["node_modules/vite/bin/vite.js", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
  {
    cwd: appDir,
    env: { ...process.env, VITE_PRODUCT_DEV_AUTH: "1" },
    stdio: ["ignore", "pipe", "pipe"],
  },
);
let devLog = "";
dev.stdout.on("data", (c) => (devLog += c));
dev.stderr.on("data", (c) => (devLog += c));

async function waitForServer() {
  for (let i = 0; i < 100; i += 1) {
    if (dev.exitCode != null) throw new Error(`vite exited:\n${devLog}`);
    try {
      if ((await fetch(url)).ok) return;
    } catch {}
    await sleep(150);
  }
  throw new Error("vite dev server did not start");
}

// Signed-in local account with a project folder, so the store's start path is
// ready (MockBridge needs no real creds).
const SEED = () => {
  localStorage.clear();
  localStorage.setItem(
    "agent-desktop.dev-account",
    JSON.stringify({
      user: { id: "profiler", name: "Profiler", email: "p@example.local", method: "local" },
    }),
  );
  localStorage.setItem(
    "agent-desktop:project-context:id%3Aprofiler",
    JSON.stringify({ cwd: "/tmp/agent-profiling" }),
  );
};

const store = `window.__clarkProfiling.store`;

// A single realistic agent turn: prose + fenced code blocks in the languages
// Shiki preloads + a markdown table. One turn ≈ 1.4k chars, ≈ 6 code fences.
const REPLY_BLOCK = [
  "Walkthrough, part N:\n\n## Step\n\nStore side:",
  "```typescript\n" +
    "export function openConversation(id: string) {\n" +
    "  const entry = liveSessions.get(id);\n" +
    "  if (entry) { set({ session: entry.session, snapshot: mergedOf(entry) }); return; }\n" +
    "  const restored = await fetchSnapshot(id, auth);\n" +
    "  await bridge.connect(provider, config);\n" +
    "  return bridge.loadSession(provider, id);\n" +
    "}\n",
  "Host side:\n",
  "```rust\n" +
    "#[tauri::command]\n" +
    "async fn open_conversation(id: String) -> Result<Session, String> {\n" +
    "    let pool = state.live_sessions.lock().await;\n" +
    "    if let Some(entry) = pool.get(&id) { return Ok(entry.session.clone()); }\n" +
    "    let restored = fetch_snapshot(&id).await?;\n" +
    "    bridge.connect(&provider, config).await?;\n" +
    "    bridge.load_session(&provider, &id).await\n" +
    "}\n",
  "Shell:\n",
  "```bash\ncargo test -p agent-core -p provider-local\npnpm --dir app build\n",
  "Config:\n",
  "```toml\n[project]\nname = \"example-desktop\"\nversion = \"0.1.0\"\n\n[dependencies]\ntauri = \"2\"\nserde = { version = \"1\", features = [\"derive\"] }\n",
  "Payload:\n",
  "```json\n{ \"session\": \"abc\", \"snapshot\": { \"timeline\": [], \"runs\": {} }, \"rev\": 42 }\n",
  "Numbers:\n\n| Path | Warm | Cold |\n| --- | --- | --- |\n| Reattach | 19ms | n/a |\n| Cold open | n/a | ~900ms |\n| Cloud fetch | 0 | ~400ms |\n\nSome closing prose. ".repeat(3),
].join("\n");

// Append N completed turns (each ≈ REPLY_BLOCK) to the ACTIVE conversation's
// store snapshot in one set, so the transcript grows to realistic size. These
// only live in the store snapshot (not entry.live), which is exactly what we
// want for measuring the RENDER cost of a heavy transcript.
async function growTranscript(page, turnCount, tag) {
  await page.evaluate(
    ({ turnCount, tag, block }) => {
      const st = window.__clarkProfiling.store;
      const snap = st.getState().snapshot;
      const timeline = [...snap.timeline];
      const runs = { ...snap.runs };
      for (let i = 0; i < turnCount; i += 1) {
        const runId = `${tag}-${i}`;
        runs[runId] = { id: runId, status: "done", outcome: { status: "done", stop_reason: "end_turn" }, checkpoint: "mock" };
        timeline.push(
          { item: "message", run: runId, role: "user", blocks: [{ type: "text", text: `turn ${i}: keep going` }] },
          { item: "message", run: runId, role: "agent", phase: "final_answer", blocks: [{ type: "text", text: block.replaceAll("part N", `part ${i}`) }] },
        );
      }
      st.setState({ snapshot: { ...snap, timeline, runs } });
    },
    { turnCount, tag, block: REPLY_BLOCK },
  );
}

const heavyPrompt = (i) =>
  `Write a long Rust + TypeScript walkthrough #${i}. Include many fenced code blocks in rust, typescript, bash, json, toml and several markdown tables. Be thorough.`;

async function buildConversation(page, turns) {
  await page.evaluate(() => window.__clarkProfiling.store.getState().startSession());
  await page.waitForFunction(
    () => {
      const s = window.__clarkProfiling.store.getState();
      return s.session?.id && !s.connecting;
    },
    null,
    { timeout: 15_000 },
  );
  const id = await page.evaluate(() => window.__clarkProfiling.store.getState().session.id);
  for (let t = 0; t < turns; t += 1) {
    await page.evaluate((p) => window.__clarkProfiling.store.getState().send(p), heavyPrompt(t + 1));
    // The mock's scripted run passes through its permission gate and finishes
    // on its own (~3.5s) — resolvePermission clears the gate only when it's
    // up; the run does not wait on it. Just drive the run to done, and clear
    // the gate if we happen to catch it.
    await page.waitForFunction(
      () => {
        const st = window.__clarkProfiling.store.getState();
        if (st.snapshot.pending_permission) st.resolvePermission("allow_once");
        const runs = Object.values(st.snapshot.runs);
        return runs.length > 0 && runs.every((r) => r.status !== "running" && r.status !== "queued");
      },
      null,
      { timeout: 90_000, polling: 100 },
    );
  }
  return id;
}

const t0 = Date.now();
const results = { samples: [], errors: [], note: "vite dev + StrictMode — upper bound, not prod" };
let browser;
try {
  await waitForServer();
  browser = await chromium.launch();
  const context = await browser.newContext({ viewport: { width: 1360, height: 880 } });
  await context.addInitScript(SEED);
  const page = await context.newPage();
  page.on("pageerror", (e) => results.errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error" || m.type() === "warning") console.log(`[console:${m.type()}] ${m.text()}`);
  });

  await page.goto(url, { waitUntil: "domcontentloaded" });
  await page.waitForFunction(() => window.__clarkProfiling?.store, null, { timeout: 30_000 });
  // Give init() a beat, then dump the store state the flow depends on.
  await page.waitForTimeout(1_500);
  // Verify the Conversation chunk is preloaded by the idle effect: a direct
  // import should resolve near-instantly if the idle preload already fired.
  const chunkMs = await page.evaluate(async () => {
    const t0 = performance.now();
    await import("/src/surfaces/Conversation.tsx");
    return Math.round((performance.now() - t0) * 10) / 10;
  });
  console.log(`[preload-check] Conversation chunk import after idle: ${chunkMs}ms`);
  results.conversationChunkMs = chunkMs;
  const dbg = await page.evaluate(() => {
    const s = window.__clarkProfiling.store.getState();
    return {
      auth: !!s.auth,
      provider: s.activeProvider,
      blocked: s.startBlockedReason?.(),
      cwd: s.localSettings?.cwd,
      bridge: !!s.bridge,
      providers: s.providers?.map((p) => p.id),
      session: s.session?.id,
      connecting: s.connecting,
      opening: s.opening,
      error: s.error,
      bodyText: document.body.innerText.replace(/\s+/g, " ").slice(0, 300),
    };
  });
  console.log("[state]", JSON.stringify(dbg, null, 1));

  console.log(`[seed] conversation A (${turns} heavy turns)…`);
  const idA = await buildConversation(page, turns);
  console.log(`[seed] A = ${idA}`);
  await page.evaluate(() => window.__clarkProfiling.store.getState().endSession());
  console.log(`[seed] conversation B (${turns} heavy turns)…`);
  const idB = await buildConversation(page, turns);
  console.log(`[seed] B = ${idB}`);
  // Grow the ACTIVE conversation (B) to a realistic heavy transcript, then let
  // Shiki warm + highlights land before we measure rendering.
  const fatTurns = Number(process.env.FAT_TURNS ?? 40);
  await growTranscript(page, fatTurns, "fatB");
  await page.waitForTimeout(2_000);
  const fatStats = await page.evaluate(() => {
    const s = window.__clarkProfiling.store.getState().snapshot;
    return {
      timeline: s.timeline.length,
      chars: s.timeline.reduce((n, i) => n + (i.item === "message" ? i.blocks.reduce((m, b) => m + (b.text?.length ?? 0), 0) : 0), 0),
      dom: document.querySelectorAll("#root *").length,
      shikiBlocks: document.querySelectorAll(".shiki-host").length,
    };
  });
  console.log("[seed] heavy transcript:", JSON.stringify(fatStats));
  results.heavy = fatStats;

  results.seed = { idA, idB, a: null, b: null };
  const stats = (id) =>
    page.evaluate((cid) => {
      const s = window.__clarkProfiling.store.getState();
      const snap = s.session?.id === cid ? s.snapshot : null;
      if (!snap) return null;
      const textLen = snap.timeline.reduce(
        (n, item) => n + (item.item === "message" ? item.blocks.reduce((m, b) => m + (b.text?.length ?? 0), 0) : 0),
        0,
      );
      return {
        timeline: snap.timeline.length,
        toolCalls: Object.keys(snap.tool_calls).length,
        runs: Object.keys(snap.runs).length,
        chars: textLen,
        dom: document.querySelectorAll("#root *").length,
      };
    }, id);

  const measureSwitch = async (targetId, label, profile) => {
    // Perceived switch = openConversation until the target session is committed
    // AND the next frame has painted.
    const switched = page.evaluate(
      (id) => new Promise((resolve, reject) => {
        const timeout = setTimeout(() => reject(new Error(`switch to ${id} timed out`)), 30_000);
        const st = window.__clarkProfiling.store.getState();
        const unsub = window.__clarkProfiling.store.subscribe((s) => {
          if (s.session?.id === id && !s.connecting) {
            clearTimeout(timeout);
            unsub();
            requestAnimationFrame(() => requestAnimationFrame(() => resolve(performance.now())));
          }
        });
        st.openConversation(id);
      }),
      targetId,
    );
    const tStart = Date.now();
    if (profile) {
      const cdp = await context.newCDPSession(page);
      await cdp.send("Profiler.enable");
      await cdp.send("Profiler.setSamplingInterval", { interval: 100 });
      await cdp.send("Profiler.start");
      await switched;
      const { profile: prof } = await cdp.send("Profiler.stop");
      await cdp.detach();
      writeFileSync(`/tmp/agent-switch-${label}.cpuprofile`, JSON.stringify(prof));
    } else {
      await switched;
    }
    const wall = Date.now() - tStart;
    const dom = await page.evaluate(() => document.querySelectorAll("#root *").length);
    results.samples.push({ label, targetId, wallMs: wall, dom });
    console.log(`  ${label}: ${wall}ms (dom=${dom})`);
  };

  results.seed.a = await stats(idA);
  // First-switch breakdown: is the ~150ms first-A-switch the lazy Conversation
  // chunk, initial Shiki grammar load, or real render work? Profile it cold
  // (before any warm-up) and total the JS busy time.
  {
    const cdp = await context.newCDPSession(page);
    await cdp.send("Profiler.enable");
    await cdp.send("Profiler.setSamplingInterval", { interval: 100 });
    await cdp.send("Profiler.start");
    await page.evaluate((id) => window.__clarkProfiling.store.getState().openConversation(id), idA);
    await page.waitForFunction((id) => window.__clarkProfiling.store.getState().session?.id === id, idA, { timeout: 15_000 });
    await page.waitForTimeout(300);
    const { profile: prof } = await cdp.send("Profiler.stop");
    await cdp.detach();
    writeFileSync("/tmp/agent-switch-first-cold.cpuprofile", JSON.stringify(prof));
    const busy = (prof.timeDeltas ?? []).reduce((a, b) => a + b, 0) / 1000;
    console.log(`[first-switch] JS busy ≈ ${busy.toFixed(0)}ms`);
    results.firstSwitchBusyMs = Math.round(busy);
  }
  // --- Render cost of the heavy transcript ---------------------------------
  // Time a forced re-render (same content, new snapshot identity) of the
  // 40-turn conversation. This isolates the pure React/markdown/shiki cost of
  // one switch's render, independent of the store plumbing.
  const renderCost = await page.evaluate(async () => {
    const st = window.__clarkProfiling.store;
    const snap = st.getState().snapshot;
    const samples = [];
    for (let i = 0; i < 6; i += 1) {
      const t0 = performance.now();
      await new Promise((resolve) => {
        requestAnimationFrame(() => {
          st.setState({ snapshot: { ...snap } });
          requestAnimationFrame(() => {
            requestAnimationFrame(() => resolve(performance.now()));
          });
        });
      });
      samples.push(Math.round((performance.now() - t0) * 10) / 10);
    }
    return { timeline: snap.timeline.length, samplesMs: samples };
  });
  console.log("RENDER-COST (heavy):", JSON.stringify(renderCost));
  results.renderCost = renderCost;
  // Now switch to A (light) then back to B (heavy) with the CPU profiler on,
  // so we capture the real reattach path while the heavy transcript is live.
  // Warm both first.
  await page.evaluate((id) => window.__clarkProfiling.store.getState().openConversation(id), idA);
  await page.waitForFunction((id) => window.__clarkProfiling.store.getState().session?.id === id, idA, { timeout: 15_000 });
  await page.evaluate((id) => window.__clarkProfiling.store.getState().openConversation(id), idB);
  await page.waitForFunction((id) => window.__clarkProfiling.store.getState().session?.id === id, idB, { timeout: 15_000 });
  for (let r = 0; r < rounds; r += 1) {
    await measureSwitch(idA, `r${r + 1}-A`, r === 0 || r === rounds - 1);
    await measureSwitch(idB, `r${r + 1}-B`, r === 0);
  }
  results.seed.b = await stats(idB);

  // Aggregate the CPU profiles into a self-time leaderboard.
  const agg = new Map();
  for (const f of ["/tmp/agent-switch-r1-A.cpuprofile", "/tmp/agent-switch-r1-B.cpuprofile", `/tmp/agent-switch-r${rounds}-A.cpuprofile`]) {
    let prof;
    try {
      prof = JSON.parse(readFileSync(f, "utf8"));
    } catch {
      continue;
    }
    const byId = new Map(prof.nodes.map((n) => [n.id, n]));
    const selfUs = new Map();
    const dt = prof.timeDeltas ?? [];
    const samples = prof.samples ?? [];
    for (let i = 0; i < samples.length; i += 1) {
      selfUs.set(samples[i], (selfUs.get(samples[i]) ?? 0) + (dt[i] ?? 0));
    }
    for (const [nodeId, us] of selfUs) {
      const n = byId.get(nodeId);
      if (!n) continue;
      const cf = n.callFrame;
      const file = cf.url.split("/").slice(-2).join("/");
      const key = `${cf.functionName || "(anon)"} @ ${file}:${cf.lineNumber + 1}`;
      agg.set(key, (agg.get(key) ?? 0) + us);
    }
  }
  results.topFunctions = [...agg.entries()]
    .sort((a, b) => b[1] - a[1])
    .slice(0, 30)
    .map(([fn, us]) => ({ fn, selfMs: Math.round(us / 100) / 10 }));

  const walls = results.samples.map((s) => s.wallMs);
  results.summary = {
    minMs: Math.min(...walls),
    medianMs: walls.sort((a, b) => a - b)[Math.floor(walls.length / 2)],
    maxMs: Math.max(...walls),
    switches: walls.length,
  };
  console.log(`\nSUMMARY: ${JSON.stringify(results.summary)}`);
  console.log("TOP SELF-TIME:");
  results.topFunctions.slice(0, 15).forEach((t) => console.log(`  ${String(t.selfMs).padStart(7)}ms  ${t.fn}`));
  if (results.errors.length) console.log("PAGE ERRORS:", results.errors.slice(0, 5));
  mkdirSync("/tmp/agent-profile", { recursive: true });
  writeFileSync("/tmp/agent-profile/results.json", JSON.stringify(results, null, 2));
  console.log(`DONE in ${((Date.now() - t0) / 1000).toFixed(0)}s — /tmp/agent-profile/results.json`);
} finally {
  await browser?.close();
  dev.kill("SIGTERM");
}
