// Deterministic UI acceptance: navigation cues must preserve the composer,
// stop between navigations, and respect reduced motion. No model calls.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { setTimeout as sleep } from "node:timers/promises";
import { chromium, webkit } from "playwright";
const root = new URL("../", import.meta.url).pathname;
const port = await new Promise((resolve) => {
  const server = createServer();
  server.listen(0, "127.0.0.1", () => {
    const port = server.address().port;
    server.close(() => resolve(port));
  });
});
const url = `http://127.0.0.1:${port}`;
const vite = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "--host", "127.0.0.1", "--port", String(port)], {
  cwd: `${root}app`, env: { ...process.env, VITE_PRODUCT_DEV_AUTH: "1" }, stdio: "ignore",
});
try {
  for (let i = 0; i < 100; i++) {
    if (vite.exitCode !== null) throw new Error("Vite exited before startup");
    try { if ((await fetch(url)).ok) break; } catch {}
    await sleep(100);
  }
  for (const [name, engine] of Object.entries({ chromium, webkit })) {
    const browser = await engine.launch({ headless: true });
    try {
      for (const reduce of [false, true]) {
        const context = await browser.newContext({ reducedMotion: reduce ? "reduce" : "no-preference" });
        await context.addInitScript(() => {
          localStorage.setItem("agent-desktop.dev-account", JSON.stringify({ user: { id: "motion-fixture", name: "Motion QA", method: "local" } }));
        });
        const page = await context.newPage();
        const errors = [];
        page.on("pageerror", (error) => errors.push(error.message));
        await page.goto(url);
        await page.getByLabel("Message Clark Code").waitFor();
        await page.waitForFunction(() => !!window.__agentDesktopStore);
        const select = (id) => page.evaluate((id) => {
          const store = window.__agentDesktopStore;
          store.setState({ session: { id, provider: "local", capabilities: {} }, opening: null,
            snapshot: { ...store.getState().snapshot, session: id, timeline: [
              { item: "message", role: "agent", blocks: [{ type: "text", text: "A settled conversation." }] },
            ] },
          });
        }, id);
        await select("a");
        await page.getByText("A settled conversation.", { exact: true }).waitFor();
        // Wait for the first navigation cue to finish before testing the switch.
        await page.waitForFunction(() => !document.querySelector("[data-workspace-stage]").getAnimations().length);
        await page.getByLabel("Message Clark Code").fill("An unsent draft");
        await page.evaluate(() => {
          window.__motionComposer = document.querySelector("textarea");
          window.__motionStage = document.querySelector("[data-workspace-stage]");
          const original = Element.prototype.animate;
          window.__motionCalls = [];
          Element.prototype.animate = function(frames, options) {
            if (this.hasAttribute("data-workspace-stage")) window.__motionCalls.push({ frames, options });
            return original.call(this, frames, options);
          };
        });
        await select("b");
        await page.waitForFunction(() => window.__motionCalls.length === 1);
        const switched = await page.evaluate(() => ({
          sameStage: window.__motionStage === document.querySelector("[data-workspace-stage]"),
          sameComposer: window.__motionComposer === document.querySelector("textarea"),
          motion: window.__motionCalls[0],
        }));
        assert.ok(switched.sameStage, "navigation remounted the workspace stage");
        // Composer is deliberately keyed to its draft owner. Switching chats
        // replaces that input; token updates within the same chat must not.
        assert.equal(await page.getByLabel("Message Clark Code").inputValue(), "");
        assert.equal(switched.motion.options.duration, reduce ? 120 : 200);
        assert.equal(switched.motion.frames.some((frame) => "transform" in frame), !reduce);
        assert.ok(switched.motion.frames.every((frame) => frame.opacity >= 0.96), "navigation flashed blank");
        await page.getByLabel("Message Clark Code").fill("Draft for conversation B");
        await page.evaluate(() => {
          window.__motionComposer = document.querySelector("textarea");
          const store = window.__agentDesktopStore;
          store.setState({ snapshot: { ...store.getState().snapshot, timeline: [
            { item: "message", role: "agent", blocks: [{ type: "text", text: "Streaming update." }] },
          ] } });
        });
        await page.getByText("Streaming update.", { exact: true }).waitFor();
        assert.ok(await page.evaluate(() => window.__motionComposer === document.activeElement), "streaming lost composer focus");
        assert.equal(await page.getByLabel("Message Clark Code").inputValue(), "Draft for conversation B");
        assert.equal(await page.evaluate(() => window.__motionCalls.length), 1, "token update restarted navigation motion");
        await page.getByRole("button", { name: "Settings", exact: true }).click();
        await page.getByRole("heading", { name: "General", exact: true }).waitFor();
        await page.locator("main").evaluate((element) => { element.scrollTop = element.scrollHeight; });
        await page.getByRole("button", { name: "About & updates", exact: true }).click();
        await page.getByRole("heading", { name: "About & updates", exact: true }).waitFor();
        assert.equal(await page.locator("main").evaluate((element) => element.scrollTop), 0);
        assert.deepEqual(errors, []);
        console.log(`${name} reducedMotion=${reduce}: stable navigation, streaming, settings scroll PASS`);
        await context.close();
      }
    } finally { await browser.close(); }
  }
} finally { vite.kill("SIGTERM"); }
