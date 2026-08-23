// Runtime motion probe for the permission-gate exit.
//
// Proves the two halves of the motion policy against the real browser:
//  - full motion: the Full-access flip still animates the gate out (~200 ms),
//    not a 1-frame hard cut;
//  - reduced motion: the gate fades out over ~120 ms with NO spatial movement
//    (transform stays "none").
//
// Boots the app (vite dev + deterministic mock bridge), drives a run that hits
// the permission gate, then flips approval to "Full access" via the composer
// pill while a rAF recorder samples the gate's computed opacity each frame.
//
// Usage: node harness/motion-exit-probe.mjs [chromium|webkit|both]
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import { webkit } from "playwright";
import { launch, VIEWPORT } from "./launch.mjs";

const repoDir = new URL("..", import.meta.url).pathname;

function reservePort() {
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

const seedLocalStorage = `
  const scope = encodeURIComponent("id:motion-qa");
  localStorage.setItem("agent-desktop.dev-account", JSON.stringify({
    user: { id: "motion-qa", name: "Motion QA", method: "local" },
  }));
  localStorage.setItem('agent-desktop:local-agent:' + scope, JSON.stringify({
    cwd: "/tmp", model: "local-model", reasoningEffort: "high",
  }));
  localStorage.setItem('agent-desktop:project-context:' + scope, JSON.stringify({ cwd: "/tmp" }));
`;

async function scenario(browser, engine, reduce) {
  const context = await browser.newContext({
    viewport: VIEWPORT,
    reducedMotion: reduce ? "reduce" : "no-preference",
  });
  await context.addInitScript(seedLocalStorage);
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push("PAGEERROR: " + (e.message ?? String(e))));
  page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });

  await page.goto(devUrl, { waitUntil: "domcontentloaded" });
  await page.getByLabel("Message Clark Code").waitFor({ state: "visible" });

  // The text-size shortcut is a deterministic path into the global Sonner
  // host. Verify the token wrapper and reduced-motion override in both engines
  // before the permission-gate scenario changes the workspace state.
  await page.getByLabel("Message Clark Code").press("Meta+=");
  await page.locator("[data-sonner-toast]").waitFor({ state: "visible" });
  const toastProbe = await page.locator("[data-sonner-toast]").evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      transitionDuration: style.transitionDuration,
      animationDuration: style.animationDuration,
      background: style.backgroundColor,
      borderRadius: style.borderRadius,
      live: element.closest("section")?.getAttribute("aria-live") ?? null,
    };
  });

  // The account default is "Approve for me" (auto), which auto-approves the
  // mock's routine edit. Pin "Ask for approval" first so every pending request
  // shows the gate; the flip to "Full access" later is then observable.
  await page.locator('[title*="Shift+Tab to cycle"]').first().click();
  await page.getByRole("menuitemradio", { name: "Ask for approval" }).click();

  // Trigger a run that lands on the permission gate.
  await page.getByLabel("Message Clark Code").fill("First turn: inspect the workspace.");
  await page.getByLabel("Message Clark Code").press("Enter");
  try {
    await page.getByRole("button", { name: "Allow once" }).waitFor({ state: "visible", timeout: 15000 });
  } catch (e) {
    await page.screenshot({ path: `target/motion-probe-${engine}-${reduce ? "reduced" : "full"}-gate-missing.png` });
    const body = await page.locator("body").innerText().catch(() => "");
    console.log(
      `[${engine}/${reduce ? "reduced" : "full"}] gate not visible; body excerpt:\n`,
      body.slice(0, 1200),
    );
    throw e;
  }

  // Install a rAF recorder on the gate's ancestor chain (opacity is not
  // inherited, so we read the animated wrapper, not the button).
  await page.evaluate(() => {
    const btn = [...document.querySelectorAll("button")].find((x) =>
      x.textContent?.includes("Allow once"));
    if (!btn) return;
    const chain = [];
    let el = btn;
    while (el && chain.length < 7) { chain.push(el); el = el.parentElement; }
    window.__motionProbe = { samples: [], startedAt: performance.now() };
    const tick = () => {
      const open = [...document.querySelectorAll("button")].some((x) =>
        x.textContent?.includes("Allow once"));
      let rec = null;
      for (let i = 0; i < chain.length; i++) {
        const anims = chain[i].getAnimations();
        if (anims.length) {
          const cs = getComputedStyle(chain[i]);
          rec = { depth: i, o: Number.parseFloat(cs.opacity), tr: cs.transform };
          break;
        }
      }
      if (!rec) {
        const cs = getComputedStyle(chain[0]);
        rec = { depth: -1, o: Number.parseFloat(cs.opacity), tr: cs.transform };
      }
      window.__motionProbe.samples.push({ t: performance.now() - window.__motionProbe.startedAt, ...rec });
      if (open && window.__motionProbe.samples.length < 400) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });

  // Flip approval to Full access — this auto-grants the pending request, so
  // Conversation unmounts the gate child and its exit animation runs.
  await page.locator('[title*="Shift+Tab to cycle"]').first().click();
  await page.getByRole("menuitemradio", { name: "Full access" }).click();
  await sleep(450);
  await page.screenshot({ path: `target/motion-probe-${engine}-${reduce ? "reduced" : "full"}.png` });

  const probe = await page.evaluate(() => window.__motionProbe ?? { samples: [] });
  const errorsNow = [...errors];
  await context.close();
  return { engine, reduce, samples: probe.samples, errors: errorsNow, toastProbe };
}

function analyze(engine, reduce, samples) {
  const opacities = samples.filter((s) => !Number.isNaN(s.o));
  const intermediate = opacities.filter((s) => s.o > 0.02 && s.o < 0.98);
  const spatial = opacities.filter((s) => s.tr && s.tr !== "none" && s.tr !== "");
  let fadeMs = 0;
  let fades = false;
  if (opacities.length >= 2 && opacities[0].o > 0.9) {
    const last = opacities[opacities.length - 2]; // last readable frame before removal
    if (last.o < 0.6) {
      fades = true;
      fadeMs = last.t - opacities[0].t;
    }
  }
  return {
    engine,
    mode: reduce ? "reduced" : "full",
    frames: opacities.length,
    intermediateFrames: intermediate.length,
    fadeCompleteMs: Math.round(fadeMs),
    fades: fades && intermediate.length >= 3,
    spatialFrames: spatial.length,
    first: opacities[0]?.o ?? NaN,
    lastRead: opacities[opacities.length - 2]?.o ?? NaN,
  };
}

const port = await reservePort();
const dev = spawn(
  "pnpm", ["--dir", "app", "dev", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
  { cwd: repoDir, env: { ...process.env, VITE_PRODUCT_DEV_AUTH: "1" }, stdio: ["ignore", "pipe", "pipe"] },
);
let devOutput = "";
const devUrl = `http://127.0.0.1:${port}/`;
dev.stdout.on("data", (c) => (devOutput += c));
dev.stderr.on("data", (c) => (devOutput += c));

async function waitForServer() {
  for (let i = 0; i < 120; i++) {
    if (dev.exitCode != null) throw new Error(`vite exited early\n${devOutput}`);
    try { if ((await fetch(`http://127.0.0.1:${port}/`)).ok) return; } catch {}
    await sleep(250);
  }
  throw new Error(`vite did not start\n${devOutput}`);
}

const requestedEngine = process.argv[2] ?? "chromium";
if (!["chromium", "webkit", "both"].includes(requestedEngine)) {
  throw new Error(`Unknown engine ${requestedEngine}; expected chromium, webkit, or both`);
}
const engines = requestedEngine === "both" ? ["chromium", "webkit"] : [requestedEngine];
let browser;
const results = [];
try {
  await waitForServer();
  for (const engine of engines) {
    browser = engine === "webkit" ? await webkit.launch() : await launch();
    for (const reduce of [false, true]) {
      const r = await scenario(browser, engine, reduce);
      r.res = analyze(r.engine, r.reduce, r.samples);
      r.errors = r.errors.filter((e) => !e.includes("deprecated")); // motion notices are warnings
      results.push(r);
    }
    await browser.close();
    browser = undefined;
  }
} finally {
  await browser?.close();
  dev.kill("SIGTERM");
}

/** The reduced-motion exit budget, matching `--dur-fast` in app/src/index.css.
 *  Reduced motion is a low-motion vocabulary, not a no-motion one: the exit is
 *  a short opacity-only fade, because a hard cut reads as a glitch. This probe
 *  used to require a 0 ms transition here, which contradicted both the source
 *  CSS and docs/gui-motion.md and left the reduced rows permanently red — a
 *  detector that always fails cannot report a regression. */
const REDUCED_EXIT_BUDGET_MS = 120;
/** Tolerance for the float round-trip through `getComputedStyle`. */
const DURATION_EPSILON_MS = 5;

/** One verdict, used for both the printed line and the exit code. These were
 *  two separate copies of the same conditions, which is how the stale contract
 *  above survived in one of them. */
function verdict(r) {
  const toastDurationMs = Number.parseFloat(r.toastProbe.transitionDuration) * 1000;
  const toastAnimationMs = Number.parseFloat(r.toastProbe.animationDuration) * 1000;
  const toastMotionValid = r.reduce
    ? toastDurationMs > 0
      && toastDurationMs <= REDUCED_EXIT_BUDGET_MS + DURATION_EPSILON_MS
      && toastAnimationMs === 0
    : toastDurationMs > 0 && toastDurationMs <= 300;
  const toastSemanticsValid = r.toastProbe.live === "polite" && r.toastProbe.borderRadius === "12px";
  const reasons = [];
  if (r.res.intermediateFrames === 0) reasons.push("no intermediate frames (hard cut)");
  if (!r.res.fades) reasons.push("did not fade out");
  if (r.reduce && r.res.spatialFrames > 0) reasons.push("spatial motion under reduced motion");
  if (!toastMotionValid) reasons.push(`toast transition ${Math.round(toastDurationMs)}ms/animation ${Math.round(toastAnimationMs)}ms outside contract`);
  if (!toastSemanticsValid) reasons.push("toast semantics (live region or radius)");
  return { toastDurationMs, bad: reasons.length > 0, reasons };
}

for (const r of results) {
  const { toastDurationMs, bad, reasons } = verdict(r);
  console.log(
    `${bad ? "FAIL" : "PASS"} ${r.res.engine}/${r.res.mode}: frames=${r.res.frames} ` +
    `intermediate=${r.res.intermediateFrames} fadeMs=${r.res.fadeCompleteMs} ` +
    `firstO=${r.res.first.toFixed(2)} lastReadO=${r.res.lastRead.toFixed(2)} ` +
    `spatialFrames=${r.res.spatialFrames} toastMs=${Math.round(toastDurationMs)} ` +
    `toastLive=${r.toastProbe.live} errors=${r.errors.length}`,
  );
  if (bad) console.log("  reasons:", reasons.join("; "));
  if (r.errors.length) console.log("  console errors:", r.errors.join(" | "));
}
process.exitCode = results.some((r) => verdict(r).bad) ? 1 : 0;
