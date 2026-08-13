import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import { launch, VIEWPORT } from "./launch.mjs";

const repoDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const outDir = path.resolve(process.env.ARTIFACT_E2E_OUTPUT_DIR ?? path.join(repoDir, "target", "artifact-delivery-smoke"));

function reservePort() {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = typeof address === "object" && address ? address.port : null;
      server.close((error) => error ? reject(error) : resolve(port));
    });
  });
}

function check(condition, message) {
  if (!condition) throw new Error(message);
}

const port = await reservePort();
const url = `http://127.0.0.1:${port}/`;
const dev = spawn("pnpm", ["--dir", "app", "dev", "--host", "127.0.0.1", "--port", String(port), "--strictPort"], {
  cwd: repoDir,
  env: { ...process.env, VITE_PRODUCT_DEV_AUTH: "1" },
  stdio: ["ignore", "pipe", "pipe"],
});
let serverOutput = "";
dev.stdout.on("data", (chunk) => { serverOutput += chunk; });
dev.stderr.on("data", (chunk) => { serverOutput += chunk; });

async function waitForServer() {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    if (dev.exitCode != null) throw new Error(`Vite exited early\n${serverOutput}`);
    try {
      if ((await fetch(url)).ok) return;
    } catch {}
    await sleep(100);
  }
  throw new Error(`Vite did not start\n${serverOutput}`);
}

let browser;
let page;
const errors = [];
const checks = [];
try {
  await mkdir(outDir, { recursive: true });
  await waitForServer();
  browser = await launch();
  const context = await browser.newContext({ viewport: VIEWPORT, acceptDownloads: true });
  await context.addInitScript(() => {
    if (!/^https?:$/.test(window.location.protocol)) return;
    const scope = encodeURIComponent("id:artifact-e2e");
    localStorage.setItem("agent-desktop.dev-account", JSON.stringify({
      user: { id: "artifact-e2e", name: "Artifact E2E", method: "local" },
    }));
    localStorage.setItem(`agent-desktop:local-agent:${scope}`, JSON.stringify({
      cwd: "/tmp", model: "local-model", reasoningEffort: "high",
    }));
    localStorage.setItem(`agent-desktop:project-context:${scope}`, JSON.stringify({ cwd: "/tmp" }));
  });
  page = await context.newPage();
  page.on("pageerror", (error) => errors.push(error.stack ?? error.message));
  page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
  await page.goto(url, { waitUntil: "networkidle" });

  const composer = page.getByLabel("Message Clark Code");
  await composer.fill("Create an artifact image and make it available inline.");
  await composer.press("Enter");
  const permission = page.getByRole("button", { name: "Allow once" });
  try {
    await permission.waitFor({ state: "visible", timeout: 2_000 });
    await permission.click();
  } catch (error) {
    if (!String(error?.message ?? error).includes("Timeout")) throw error;
  }

  const inlineImage = page.getByRole("img", { name: "artifact-preview.svg" });
  await inlineImage.waitFor({ state: "visible" });
  check(await inlineImage.evaluate((image) => image.complete && image.naturalWidth > 0), "inline artifact image did not decode");
  checks.push("inline_image_decoded");

  const chatActions = page.getByLabel("Actions for artifact-preview.svg").first();
  for (const label of ["Save a Copy"]) {
    check(await chatActions.getByRole("button", { name: label }).count() === 1, `missing inline ${label} action`);
  }
  await page.screenshot({ path: path.join(outDir, "01-inline-chat.png"), animations: "disabled" });
  checks.push("inline_actions_visible");

  const downloadPromise = page.waitForEvent("download");
  await chatActions.getByRole("button", { name: "Save a Copy" }).click();
  const download = await downloadPromise;
  const downloadPath = await download.path();
  check(download.suggestedFilename() === "artifact-preview.svg", "download did not preserve filename");
  check(Boolean(downloadPath) && (await stat(downloadPath)).size > 0, "downloaded artifact is empty");
  checks.push("save_copy_downloaded");

  await page.getByRole("button", { name: "View artifact-preview.svg" }).click();
  const workspace = page.getByRole("region", { name: "Artifact workspace" });
  await workspace.waitFor({ state: "visible" });
  const workspaceImage = workspace.getByRole("img", { name: "artifact-preview.svg" });
  await workspaceImage.waitFor({ state: "visible" });
  check(await workspaceImage.evaluate((image) => image.complete && image.naturalWidth > 0), "workspace image did not decode");
  check(await workspace.getByRole("button", { name: "Save a Copy" }).count() === 1, "workspace download action is missing");
  await page.screenshot({ path: path.join(outDir, "02-artifact-workspace.png"), animations: "disabled" });
  checks.push("workspace_preview_and_actions");

  await page.getByRole("button", { name: "View Research summary.pdf" }).click();
  const pdfPage = workspace.getByRole("img", { name: "Research summary.pdf, page 1" });
  await pdfPage.waitFor({ state: "visible" });
  check(
    await pdfPage.evaluate((image) => image.complete && image.naturalWidth > 0),
    "PDF workspace did not rasterize the real PDF payload",
  );
  check(
    await workspace.getByText("Preview unavailable", { exact: true }).count() === 0,
    "PDF fell back to an unavailable preview",
  );
  const pdfActions = workspace.getByLabel("Actions for Research summary.pdf");
  check(await pdfActions.getByRole("button", { name: "Save a Copy" }).count() === 1, "PDF save action is missing");
  await page.screenshot({ path: path.join(outDir, "03-pdf-workspace.png"), animations: "disabled" });
  checks.push("pdf_preview_and_actions");

  const pdfDownloadPromise = page.waitForEvent("download");
  await pdfActions.getByRole("button", { name: "Save a Copy" }).click();
  const pdfDownload = await pdfDownloadPromise;
  const pdfDownloadPath = await pdfDownload.path();
  check(pdfDownload.suggestedFilename() === "Research summary.pdf", "PDF download did not preserve filename");
  check(Boolean(pdfDownloadPath), "PDF download did not produce a file");
  const pdfBytes = await readFile(pdfDownloadPath);
  check(pdfBytes.subarray(0, 5).toString() === "%PDF-", "downloaded PDF payload is invalid");
  checks.push("pdf_save_copy_downloaded");

  check(errors.length === 0, `browser errors:\n${errors.join("\n")}`);
  const receipt = {
    schema_version: 1,
    benchmark: "artifact_delivery_ui_smoke",
    status: "passed",
    mode: "mock_provider_no_paid_calls",
    checks,
    browser_console_errors: errors,
    screenshots: ["01-inline-chat.png", "02-artifact-workspace.png", "03-pdf-workspace.png"],
  };
  await writeFile(path.join(outDir, "receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.log(JSON.stringify({ ...receipt, output_dir: outDir }));
} catch (error) {
  const receipt = {
    schema_version: 1,
    benchmark: "artifact_delivery_ui_smoke",
    status: "failed",
    mode: "mock_provider_no_paid_calls",
    checks,
    browser_console_errors: errors,
    failure: String(error?.stack ?? error),
    body_text: page ? (await page.locator("body").innerText().catch(() => "")).slice(-5000) : "",
  };
  await mkdir(outDir, { recursive: true });
  await writeFile(path.join(outDir, "receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.error(JSON.stringify({ ...receipt, output_dir: outDir }));
  process.exitCode = 1;
} finally {
  await browser?.close();
  dev.kill("SIGTERM");
}
