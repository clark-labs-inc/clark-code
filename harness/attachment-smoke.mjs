import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { webkit } from "playwright";

const root = fileURLToPath(new URL("..", import.meta.url));
const port = Number(process.env.CLARK_ATTACHMENT_SMOKE_PORT ?? 4175);
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

function check(value, message) {
  if (!value) throw new Error(message);
}

let browser;
try {
  await waitForPreview();
  browser = await webkit.launch();
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  await context.addInitScript(() => {
    localStorage.setItem(
      "clark.auth.session",
      JSON.stringify({
        user: { name: "Attachment QA", method: "local" },
        clark: { endpoint: "ws://localhost:8400/ws" },
      }),
    );
    localStorage.setItem(
      "clark-desktop:local-agent",
      JSON.stringify({ cwd: "/tmp", model: "clark-code", reasoningEffort: "", apiKey: "" }),
    );
  });

  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.stack ?? error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  await page.goto(url, { waitUntil: "networkidle" });

  const composer = page.getByLabel("Message Clark");
  await composer.fill("Initialize the attachment smoke session.");
  await composer.press("Enter");
  await page.waitForTimeout(3500);

  const photo =
    "/tmp/codex-remote-attachments/019f72ef-b16e-7b53-96c4-1f3e3b9a309b/" +
    "D96318F6-CC4C-459B-A64C-8D6DBF9CFFC7/1-Photo-1.jpg";
  await page.setInputFiles('input[type="file"]', photo);
  await page.waitForTimeout(300);

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
  check((await composer.inputValue()) === "", "large paste leaked into the textarea");
  check(
    await page.getByText(`Pasted Content ${Array.from(paste).length} chars`, { exact: true }).isVisible(),
    "large-paste thumbnail chip is missing",
  );
  check(await page.locator('[role="listitem"] img').isVisible(), "image thumbnail is missing");
  await page.screenshot({ path: "/tmp/clark-desktop-attachment-smoke.png" });

  await composer.press("Enter");
  await page.waitForTimeout(250);
  const body = await page.locator("body").innerText();
  check(body.includes("LARGE_PASTE_UI_BEGIN"), "expanded paste beginning did not reach the turn");
  check(body.includes("LARGE_PASTE_UI_END"), "expanded paste ending did not reach the turn");
  check(!body.includes(placeholder), "composer placeholder leaked into the submitted turn");
  check((await page.locator('[aria-label="Attachments"]').count()) === 0, "chips did not clear");
  check(errors.length === 0, `browser errors:\n${errors.join("\n")}`);

  console.log(
    JSON.stringify({
      eval: "attachment_ui",
      mode: "webkit_mock_desktop",
      status: "pass",
      checks: 7,
      screenshot: "/tmp/clark-desktop-attachment-smoke.png",
    }),
  );
} finally {
  await browser?.close();
  preview.kill("SIGTERM");
}
