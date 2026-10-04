// Browser contracts for settled history, streaming completion, and disclosure
// scroll ownership. Uses the real components with deterministic store input.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { createServer } from "node:net";
import { setTimeout as sleep } from "node:timers/promises";
import { chromium, webkit } from "playwright";

const root = new URL("../", import.meta.url).pathname;
const port = await new Promise((resolve, reject) => {
  const server = createServer();
  server.once("error", reject);
  server.listen(0, "127.0.0.1", () => {
    const { port } = server.address();
    server.close(() => resolve(port));
  });
});
const url = `http://127.0.0.1:${port}`;
const vite = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "--host", "127.0.0.1", "--port", String(port), "--strictPort"], {
  cwd: `${root}app`, env: { ...process.env, VITE_PRODUCT_DEV_AUTH: "1" }, stdio: "ignore",
});
const failures = [];
try {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (vite.exitCode !== null) throw new Error("Vite exited before startup");
    try { if ((await fetch(url)).ok) break; } catch {}
    await sleep(100);
  }
  for (const [name, engine] of Object.entries({ chromium, webkit })) {
    if (process.env.ENGINE && process.env.ENGINE !== name) continue;
    const browser = await engine.launch({ headless: true });
    try {
      for (const reducedMotion of ["no-preference", "reduce"]) {
        const context = await browser.newContext({ viewport: { width: 1360, height: 880 }, reducedMotion });
        await context.addInitScript(() => {
          localStorage.setItem("agent-desktop.dev-account", JSON.stringify({ user: { id: "rendering-qa", name: "Rendering QA", method: "local" } }));
        });
        const page = await context.newPage();
        const errors = [];
        page.on("pageerror", (error) => errors.push(error.message));
        await page.goto(url);
        await page.getByLabel("Message Clark Code").waitFor();
        // Warm the lazy conversation before measuring its first rendered frame.
        await page.evaluate(async () => { await import("/src/surfaces/Conversation.tsx"); });
        const history = await page.evaluate(async () => {
          const store = window.__agentDesktopStore;
          const calls = Array.from({ length: 60 }, (_, index) => ({
            id: `receipt-${index}`, title: `Result ${index}`, kind: index % 10 === 0 ? "research" : "execute", status: "completed",
            content: [{ type: "text", text: Array.from({ length: 40 }, (_, line) => `output ${line}`).join("\n") }],
            locations: [],
          }));
          store.setState({
            session: { id: "receipts", provider: "local", capabilities: {} }, opening: null,
            snapshot: { ...store.getState().snapshot, session: "receipts", runs: {},
              timeline: calls.map((call) => ({ item: "tool_call", id: call.id, run: "done" })),
              tool_calls: Object.fromEntries(calls.map((call) => [call.id, call])),
            },
          });
          const deadline = performance.now() + 5000;
          while (!document.querySelector("[data-tool-call-id]") && performance.now() < deadline) {
            await new Promise(requestAnimationFrame);
          }
          const rows = [...document.querySelectorAll("[data-tool-call-id]")];
          return { mounted: rows.length, faded: rows.filter((row) => Number(getComputedStyle(row).opacity) < 0.99).length };
        });
        await page.locator('[data-tool-call-id="receipt-59"]').waitFor();
        await sleep(350);
        await page.waitForFunction(() => {
          const log = document.querySelector('[role="log"]');
          return log && Math.abs(log.scrollHeight - log.clientHeight - log.scrollTop) < 2;
        });
        const disclosure = page.locator('[data-tool-call-id="receipt-59"] > button');
        const beforeTop = await disclosure.evaluate((element) => element.getBoundingClientRect().top);
        await disclosure.click();
        await sleep(300);
        const disclosureShift = await disclosure.evaluate((element, before) => element.getBoundingClientRect().top - before, beforeTop);

        const appendOutput = (lines) => page.evaluate((lines) => {
          const store = window.__agentDesktopStore;
          const snapshot = store.getState().snapshot;
          store.setState({ snapshot: { ...snapshot,
            runs: { live: { id: "live", status: "running" } },
            timeline: [
              ...snapshot.timeline.filter((item) => item.item === "tool_call"),
              { item: "message", role: "agent", phase: "final_answer", run: "live",
                blocks: [{ type: "text", text: Array.from({ length: lines }, (_, i) => `Output paragraph ${i}.`).join("\n\n") }] },
            ],
          } });
        }, lines);
        const log = page.getByRole("log");
        const readingTop = await log.evaluate((element) => element.scrollTop);
        await appendOutput(30);
        await page.getByText("Output paragraph 29.", { exact: true }).waitFor();
        await sleep(300);
        const readingShift = await log.evaluate((element, before) => element.scrollTop - before, readingTop);
        await page.getByRole("button", { name: "Jump to latest" }).click();
        const waitForBottom = () => page.waitForFunction(() => {
          const element = document.querySelector('[role="log"]');
          return Math.abs(element.scrollHeight - element.clientHeight - element.scrollTop) < 2;
        });
        await waitForBottom();
        await appendOutput(40);
        await page.getByText("Output paragraph 39.", { exact: true }).waitFor();
        await waitForBottom();
        await log.hover();
        await page.mouse.wheel(0, -350);
        await page.getByRole("button", { name: "Jump to latest" }).waitFor();
        await sleep(200);
        const scrollbackTop = await log.evaluate((element) => element.scrollTop);
        await appendOutput(50);
        await page.getByText("Output paragraph 49.", { exact: true }).waitFor();
        await sleep(300);
        const scrollbackShift = await log.evaluate((element, before) => element.scrollTop - before, scrollbackTop);

        const setAnswer = (text, running) => page.evaluate(({ text, running }) => {
          const store = window.__agentDesktopStore;
          store.setState({
            session: { id: "answer", provider: "local", capabilities: {} }, opening: null,
            snapshot: { ...store.getState().snapshot, session: "answer", tool_calls: {},
              timeline: [
                { item: "message", role: "user", run: "answer-run", blocks: [{ type: "text", text: "Show the result" }] },
                { item: "message", role: "agent", phase: "final_answer", run: "answer-run", blocks: [{ type: "text", text }] },
              ],
              runs: { "answer-run": { id: "answer-run", status: running ? "running" : "done" } },
            },
          });
        }, { text, running });
        const code = "```\nA stable code fence\n```\n\nResult explained.";
        await setAnswer(code, true);
        await page.getByText("A stable code fence", { exact: false }).waitFor();
        await page.evaluate(() => { window.__renderingFence = document.querySelector('[role="log"] pre'); });
        await setAnswer(code, false);
        await sleep(100);
        const retainedFence = await page.evaluate(() => window.__renderingFence === document.querySelector('[role="log"] pre'));

        await setAnswer("Done.", true);
        await sleep(350);
        const completion = await page.evaluate(async () => {
          const store = window.__agentDesktopStore;
          const frame = document.querySelector(".streaming-reply-frame");
          const heights = [];
          const initial = frame.getBoundingClientRect().height;
          store.setState({ snapshot: { ...store.getState().snapshot, runs: { "answer-run": { id: "answer-run", status: "done" } } } });
          const end = performance.now() + 450;
          while (performance.now() < end) {
            await new Promise(requestAnimationFrame);
            heights.push(frame.getBoundingClientRect().height);
          }
          const final = heights.at(-1);
          return { initial, final, intermediate: heights.filter((height) => height > final + 1 && height < initial - 1).length };
        });
        if (name === "chromium" && reducedMotion === "no-preference") {
          mkdirSync(`${root}target/ui-rendering`, { recursive: true });
          await page.screenshot({ path: `${root}target/ui-rendering/settled.png` });
        }
        const label = `${name} ${reducedMotion}`;
        console.log(JSON.stringify({ label, history, disclosureShift, readingShift, scrollbackShift, retainedFence, completion, errors }));
        if (history.mounted !== 60 || history.faded !== 0) failures.push(`${label}: saved tool history replayed entrance motion`);
        if (Math.abs(disclosureShift) > 2) failures.push(`${label}: expanding a tool moved its header ${disclosureShift.toFixed(1)}px`);
        if (Math.abs(readingShift) > 2) failures.push(`${label}: new output pulled the reader away from the expanded tool`);
        if (Math.abs(scrollbackShift) > 2) failures.push(`${label}: new output interrupted scrollback`);
        if (!retainedFence) failures.push(`${label}: completion remounted the code fence`);
        if (completion.intermediate > 1) failures.push(`${label}: completion animated transcript height across ${completion.intermediate} frames`);
        if (errors.length) failures.push(`${label}: ${errors.join("; ")}`);
        await context.close();
      }
    } finally { await browser.close(); }
  }
} finally { vite.kill("SIGTERM"); }
assert.deepEqual(failures, []);
