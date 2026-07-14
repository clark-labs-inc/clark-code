import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { webkit } from "playwright";

const root = fileURLToPath(new URL("..", import.meta.url));
const port = Number(process.env.CLARK_WEBKIT_SMOKE_PORT ?? 4174);
const url = `http://127.0.0.1:${port}/`;
const preview = spawn(
  "pnpm",
  ["--dir", "app", "preview", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
  { cwd: root, stdio: ["ignore", "pipe", "pipe"] },
);
let previewOutput = "";
preview.stdout.on("data", (chunk) => (previewOutput += chunk));
preview.stderr.on("data", (chunk) => (previewOutput += chunk));

async function waitForPreview() {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    if (preview.exitCode != null) throw new Error(`preview exited early\n${previewOutput}`);
    try {
      const response = await fetch(url);
      if (response.ok) return;
    } catch {}
    await sleep(100);
  }
  throw new Error(`preview did not start\n${previewOutput}`);
}

async function verifyStartup(browser, authenticated) {
  const context = await browser.newContext();
  if (authenticated) {
    await context.addInitScript(() => {
      localStorage.setItem(
        "clark.auth.session",
        JSON.stringify({
          user: { name: "WebKit QA", method: "local" },
          clark: { endpoint: "ws://localhost:8400/ws" },
        }),
      );
      localStorage.setItem(
        "clark-desktop:local-agent",
        JSON.stringify({ cwd: "/tmp", model: "clark-code", reasoningEffort: "", apiKey: "" }),
      );
    });
  }

  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.stack ?? error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  await page.goto(url, { waitUntil: "networkidle" });
  await page.waitForTimeout(250);
  const state = await page.evaluate(() => ({
    rootChildren: document.querySelector("#root")?.childElementCount ?? 0,
    text: document.body.innerText,
  }));
  await context.close();

  if (errors.length > 0) throw new Error(errors.join("\n"));
  if (state.rootChildren === 0) throw new Error("React did not mount");
  const expected = authenticated ? "New session" : "Continue with Google";
  if (!state.text.includes(expected)) throw new Error(`missing startup text: ${expected}`);
}

let browser;
try {
  await waitForPreview();
  browser = await webkit.launch();
  await verifyStartup(browser, false);
  await verifyStartup(browser, true);
  console.log("WebKit production startup passed (signed out + authenticated)");
} finally {
  await browser?.close();
  preview.kill("SIGTERM");
}
