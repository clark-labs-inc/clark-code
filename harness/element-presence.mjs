// Real modal lifecycle, sampled at every animation frame. Synthetic local state;
// no native app restart, network account, provider, or model calls.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync, copyFileSync } from "node:fs";
import { createServer } from "node:net";
import { setTimeout as sleep } from "node:timers/promises";
import { chromium, webkit } from "playwright";

const root = new URL("../", import.meta.url).pathname;
const output = process.env.OUTPUT ?? `${root}target/element-presence`;
mkdirSync(output, { recursive: true });
const port = await new Promise((resolve, reject) => {
  const server = createServer();
  server.once("error", reject);
  server.listen(0, "127.0.0.1", () => {
    const port = server.address().port;
    server.close(() => resolve(port));
  });
});
const url = `http://127.0.0.1:${port}`;
const vite = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "--host", "127.0.0.1", "--port", String(port), "--strictPort"], {
  cwd: `${root}app`, env: { ...process.env, VITE_PRODUCT_DEV_AUTH: "1" }, stdio: "ignore",
});
const cases = [
  ["palette", "setPaletteOpen", '[aria-label="Command palette"]'],
  ["session", "setNewProjectOpen", '[aria-labelledby="new-project-title"]'],
  ["settings", "setSettingsOpen", '[aria-labelledby="settings-title"]'],
  ["mcp", "setMcpOpen", '[aria-labelledby="mcp-settings-title"]'],
  ["ssh", "setSshOpen", '[aria-labelledby="ssh-settings-title"]'],
];
const results = [];
const failures = [];
try {
  for (let i = 0; i < 100; i++) {
    if (vite.exitCode !== null) throw new Error("Vite exited before startup");
    try { if ((await fetch(url)).ok) break; } catch {}
    await sleep(100);
  }
  for (const [engineName, engine] of Object.entries({ chromium, webkit })) {
    if (process.env.ENGINE && process.env.ENGINE !== engineName) continue;
    const browser = await engine.launch({ headless: true });
    try {
      for (const reducedMotion of ["no-preference", "reduce"]) {
        const capture = (engineName === "chromium" && reducedMotion === "no-preference")
          || (engineName === "webkit" && reducedMotion === "reduce");
        const context = await browser.newContext({ viewport: { width: 1360, height: 880 }, reducedMotion,
          ...(capture ? { recordVideo: { dir: `${output}/video`, size: { width: 1360, height: 880 } } } : {}),
        });
        await context.addInitScript(() => {
          localStorage.setItem("agent-desktop.dev-account", JSON.stringify({ user: { id: "presence-qa", name: "Presence QA", method: "local" } }));
          localStorage.setItem("agent-desktop.theme", "dark");
        });
        const page = await context.newPage();
        const errors = [];
        page.on("pageerror", (error) => errors.push(error.message));
        await page.goto(url);
        const composer = page.getByLabel("Message Clark Code");
        await composer.waitFor();
        await composer.fill("Keep this draft and its focus across modal transitions.");
        const measured = [];
        for (const [name, setter, selector] of cases) {
          // Cold open through the same store action as the real control. A lazy
          // chunk may load here; settled exits below do not include chunk delay.
          const entry = await page.evaluate(async ({ setter, selector }) => {
            const samples = [];
            const start = performance.now();
            let node = null;
            let mountedAt = null;
            window.__agentDesktopStore.getState()[setter](true);
            while (performance.now() - (mountedAt ?? start) < (mountedAt === null ? 5000 : 400)) {
              await new Promise(requestAnimationFrame);
              const current = document.querySelector(selector);
              if (!current) continue;
              if (!node) { node = current; mountedAt = performance.now(); }
              const box = current.getBoundingClientRect();
              samples.push({ t: performance.now() - start, sameNode: current === node,
                count: document.querySelectorAll(selector).length,
                opacity: Number(getComputedStyle(current).opacity), x: box.x, y: box.y,
                inlineOpacity: current.style.opacity,
                animations: current.getAnimations().map((animation) => ({ type: animation.constructor.name, property: animation.transitionProperty ?? null })),
              });
            }
            return { samples, mountedAt: mountedAt === null ? null : mountedAt - start,
              reversals: samples.filter((s, i) => i && s.opacity < samples[i - 1].opacity - 0.02).length,
            };
          }, { setter, selector });
          // Warm close is where a destroyed presence owner loses exit frames.
          const exit = await page.evaluate(async ({ setter, selector }) => {
            const node = document.querySelector(selector);
            const samples = [];
            const start = performance.now();
            const sample = () => {
              const style = getComputedStyle(node);
              samples.push({ t: performance.now() - start, connected: node.isConnected,
                opacity: node.isConnected ? Number(style.opacity) : null,
                transform: style.transform, inert: node.inert,
              });
            };
            sample();
            window.__agentDesktopStore.getState()[setter](false);
            while (performance.now() - start < 360) {
              await new Promise(requestAnimationFrame);
              sample();
            }
            return { samples,
              intermediate: samples.filter((s) => s.connected && s.opacity > 0.02 && s.opacity < 0.98).length,
              removedAt: samples.find((s) => !s.connected)?.t ?? null,
            };
          }, { setter, selector });
          // Rapid close/reopen must reverse the same live node, not flash back
          // through a newly mounted opacity-zero tree.
          const rapid = await page.evaluate(async ({ setter, selector }) => {
            const store = window.__agentDesktopStore;
            store.getState()[setter](true);
            await new Promise((resolve) => setTimeout(resolve, 300));
            const node = document.querySelector(selector);
            const before = node.getBoundingClientRect();
            store.getState()[setter](false);
            await new Promise((resolve) => setTimeout(resolve, 50));
            const retainedDuringExit = node.isConnected;
            store.getState()[setter](true);
            const samples = [];
            const start = performance.now();
            while (performance.now() - start < 320) {
              await new Promise(requestAnimationFrame);
              const current = document.querySelector(selector);
              const box = current?.getBoundingClientRect();
              samples.push({ t: performance.now() - start, sameNode: current === node,
                count: document.querySelectorAll(selector).length,
                opacity: current ? Number(getComputedStyle(current).opacity) : null,
                x: box?.x, y: box?.y,
              });
            }
            return { retainedDuringExit, sameNode: samples.every((s) => s.sameNode), samples,
              settledDisplacement: samples.length ? Math.hypot(samples.at(-1).x - before.x, samples.at(-1).y - before.y) : null,
            };
          }, { setter, selector });
          if (capture && name === "session") await page.screenshot({ path: `${output}/session.png` });
          await page.evaluate(({ setter }) => window.__agentDesktopStore.getState()[setter](false), { setter });
          await page.locator(selector).waitFor({ state: "detached" });
          const row = { name, entry, exit, rapid };
          measured.push(row);
          if (entry.mountedAt === null || entry.reversals || entry.samples.at(-1)?.opacity < 0.99
            || entry.samples.some((s) => !s.sameNode || s.count !== 1 || s.animations.some((a) => a.property === "opacity"))) failures.push(`${engineName}/${reducedMotion}/${name}: entry flickered or remounted`);
          if (!exit.intermediate || exit.removedAt === null || exit.removedAt < 60) failures.push(`${engineName}/${reducedMotion}/${name}: exit hard cut`);
          if (!rapid.retainedDuringExit || !rapid.sameNode || rapid.samples.some((s) => s.count !== 1) || rapid.settledDisplacement > 1) failures.push(`${engineName}/${reducedMotion}/${name}: rapid reopen replaced or displaced the dialog`);
        }
        assert.equal(await composer.inputValue(), "Keep this draft and its focus across modal transitions.");
        const result = { engine: engineName, reducedMotion, cases: measured, errors };
        results.push(result);
        console.log(JSON.stringify({ engine: engineName, reducedMotion, cases: measured.map(({ name, entry, exit, rapid }) => ({ name, entryReversals: entry.reversals, exitFrames: exit.intermediate, removedAt: exit.removedAt, retainedDuringExit: rapid.retainedDuringExit, sameNode: rapid.sameNode, settledDisplacement: rapid.settledDisplacement })), errors }));
        if (errors.length) failures.push(`${engineName}/${reducedMotion}: ${errors.join("; ")}`);
        const video = page.video();
        await context.close();
        if (capture) copyFileSync(await video.path(), `${output}/${engineName}-${reducedMotion}.webm`);
      }
    } finally { await browser.close(); }
  }
} finally {
  vite.kill("SIGTERM");
  writeFileSync(`${output}/frames.json`, JSON.stringify({ results, failures }, null, 2));
}
assert.deepEqual(failures, []);
