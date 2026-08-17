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
        else if (port == null) reject(new Error("failed to reserve an attachment smoke port"));
        else resolve(port);
      });
    });
  });
}

const port = process.env.AGENT_ATTACHMENT_SMOKE_PORT
  ? Number(process.env.AGENT_ATTACHMENT_SMOKE_PORT)
  : await availablePort();
const url = `http://127.0.0.1:${port}/`;
const preview = spawn(
  "pnpm",
  ["--dir", "app", "dev", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
  {
    cwd: root,
    env: { ...process.env, VITE_PRODUCT_DEV_AUTH: "1" },
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

function check(value, message) {
  if (!value) throw new Error(message);
}

let browser;
try {
  await waitForPreview();
  browser = await webkit.launch();
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  await context.addInitScript(() => {
    const accountScope = "id:attachment-qa";
    const encodedScope = encodeURIComponent(accountScope);
    localStorage.setItem(
      "agent-desktop.dev-account",
      JSON.stringify({
        user: { id: "attachment-qa", name: "Attachment QA", method: "local" },
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

  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.stack ?? error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  await page.goto(url, { waitUntil: "networkidle" });

  const composer = page.getByLabel("Message Clark Code");
  try {
    await composer.waitFor({ state: "visible", timeout: 5_000 });
  } catch (error) {
    const text = await page.locator("body").innerText();
    throw new Error(
      `Clark Code composer did not appear; body=${JSON.stringify(text)}; ` +
        `page_errors=${JSON.stringify(errors)}; cause=${error}`,
    );
  }
  await composer.fill("Initialize the attachment smoke session.");
  await composer.press("Enter");
  await page.waitForTimeout(3500);

  await page.setInputFiles('input[type="file"]', {
    name: "attachment-smoke.png",
    mimeType: "image/png",
    buffer: Buffer.from(
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
      "base64",
    ),
  });
  await page.waitForTimeout(300);
  const submittedText = "Explain which interface macOS will use for this route.";
  await composer.fill(submittedText);

  const paste = `LARGE_PASTE_UI_BEGIN-${"x".repeat(1_001)}-LARGE_PASTE_UI_END`;
  await composer.evaluate((element, text) => {
    const clipboard = new DataTransfer();
    clipboard.setData("text/plain", text);
    element.dispatchEvent(
      new ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData: clipboard }),
    );
  }, paste);
  await page.waitForTimeout(200);

  const placeholder = `[Pasted Content ${Array.from(paste).length} chars]`;
  check(
    (await composer.inputValue()) === submittedText,
    "large paste leaked into the textarea",
  );
  check(
    await page.getByText(`Pasted Content ${Array.from(paste).length} chars`, { exact: true }).isVisible(),
    "large-paste thumbnail chip is missing",
  );
  check(await page.locator('[role="listitem"] img').isVisible(), "image thumbnail is missing");
  await page.screenshot({ path: "/tmp/agent-desktop-attachment-smoke.png" });

  await composer.press("Enter");
  await page.waitForTimeout(50);
  check((await composer.inputValue()) === "", "submitted text remained in the composer");
  check(await page.getByLabel("Sending message").isVisible(), "sending state is missing");
  check(
    (await page.locator('[aria-label="Attachments"]').count()) === 0,
    "chips did not clear during admission",
  );
  await page.waitForTimeout(500);
  const body = await page.locator("body").innerText();
  check(body.includes("LARGE_PASTE_UI_BEGIN"), "expanded paste beginning did not reach the turn");
  check(body.includes("LARGE_PASTE_UI_END"), "expanded paste ending did not reach the turn");
  check(!body.includes(placeholder), "composer placeholder leaked into the submitted turn");
  check(
    (await page.getByLabel("Sending message").count()) === 0,
    "sending state did not settle",
  );
  check(errors.length === 0, `browser errors:\n${errors.join("\n")}`);

  console.log(
    JSON.stringify({
      eval: "attachment_ui",
      mode: "webkit_mock_desktop",
      status: "pass",
      checks: 11,
      screenshot: "/tmp/agent-desktop-attachment-smoke.png",
    }),
  );
} finally {
  await browser?.close();
  preview.kill("SIGTERM");
}
