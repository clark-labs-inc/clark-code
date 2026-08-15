import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { webkit } from "playwright";

const root = fileURLToPath(new URL("..", import.meta.url));
async function availablePort() {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = typeof address === "object" && address ? address.port : null;
      server.close((error) => {
        if (error) reject(error);
        else if (port == null) reject(new Error("failed to reserve a WebKit smoke port"));
        else resolve(port);
      });
    });
  });
}

const port = process.env.AGENT_WEBKIT_SMOKE_PORT
  ? Number(process.env.AGENT_WEBKIT_SMOKE_PORT)
  : await availablePort();
const url = `http://127.0.0.1:${port}/`;
const preview = spawn(
  "pnpm",
  ["--dir", "app", "dev", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
  {
    cwd: root,
    env: { ...process.env, VITE_PRODUCT_DEV_AUTH: "0" },
    stdio: ["ignore", "pipe", "pipe"],
  },
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
      const accountScope = "id:webkit-qa";
      const encodedScope = encodeURIComponent(accountScope);
      localStorage.setItem(
        "agent-desktop.dev-account",
        JSON.stringify({
          user: { id: "webkit-qa", name: "WebKit QA", method: "local" },
        }),
      );
      localStorage.setItem(
        `agent-desktop:local-agent:${encodedScope}`,
        JSON.stringify({
          cwd: "",
          model: "local-model",
          reasoningEffort: "high",
        }),
      );
      localStorage.setItem(
        `agent-desktop:project-context:${encodedScope}`,
        JSON.stringify({ cwd: "/tmp" }),
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
  // The neutral foundation has no branded authentication gate. Exercise both
  // an empty browser profile and a restored local profile, but require the
  // same provider-neutral startup surface in each case.
  const expected = "New session";
  try {
    await page.waitForFunction(
      (text) => document.body.innerText.includes(text),
      expected,
      { timeout: 5_000 },
    );
  } catch (error) {
    const text = await page.locator("body").innerText();
    throw new Error(
      `startup text ${JSON.stringify(expected)} did not appear; body=${JSON.stringify(text)}; ` +
        `page_errors=${JSON.stringify(errors)}; cause=${error}`,
    );
  }
  const state = await page.evaluate(() => ({
    rootChildren: document.querySelector("#root")?.childElementCount ?? 0,
    text: document.body.innerText,
  }));
  await context.close();

  if (errors.length > 0) throw new Error(errors.join("\n"));
  if (state.rootChildren === 0) throw new Error("React did not mount");
  if (!state.text.includes(expected)) throw new Error(`missing startup text: ${expected}`);
}

let browser;
try {
  await waitForPreview();
  browser = await webkit.launch();
  await verifyStartup(browser, false);
  await verifyStartup(browser, true);
  console.log("WebKit foundation startup passed (empty + restored profile)");
} finally {
  await browser?.close();
  preview.kill("SIGTERM");
}
