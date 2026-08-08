import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import { launch, VIEWPORT } from "./launch.mjs";

const repoDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
const outDir = path.join(repoDir, "target", "full-gui-smoke", `${stamp}-${process.pid}`);

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

function check(condition, message) {
  if (!condition) throw new Error(message);
}

async function approveIfNeeded(page) {
  const button = page.getByRole("button", { name: "Allow once" });
  try {
    await button.waitFor({ state: "visible", timeout: 2_000 });
    await button.click();
  } catch (error) {
    if (!String(error?.message ?? error).includes("Timeout")) throw error;
  }
}

const expectedSlashCommands = [
  "new", "goal", "compact", "skills", "scout", "security", "security-diff",
  "security-deep", "scientist", "rsi", "sentry", "terminal", "mcp", "copy",
  "share", "unshare", "memory", "btw",
];

let browser;
let page;
const errors = [];
const checks = [];
try {
  await mkdir(outDir, { recursive: true });
  await waitForServer();
  browser = await launch();
  const context = await browser.newContext({ viewport: VIEWPORT });
  await context.addInitScript(() => {
    const accountScope = "id:full-gui-qa";
    const encodedScope = encodeURIComponent(accountScope);
    localStorage.setItem("agent-desktop.dev-account", JSON.stringify({
      user: { id: "full-gui-qa", name: "Full GUI QA", method: "local" },
    }));
    localStorage.setItem(`agent-desktop:local-agent:${encodedScope}`, JSON.stringify({
      cwd: "/tmp", model: "local-model", reasoningEffort: "high",
    }));
    localStorage.setItem(`agent-desktop:project-context:${encodedScope}`, JSON.stringify({ cwd: "/tmp" }));
  });
  page = await context.newPage();
  page.on("pageerror", (error) => errors.push(error.stack ?? error.message));
  page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
  await page.goto(url, { waitUntil: "networkidle" });
  const composer = page.getByLabel("Message Agent Desktop");
  await composer.waitFor({ state: "visible" });

  // Multi-turn conversation with the deterministic mock, including permission.
  await composer.fill("First turn: inspect the workspace.");
  await composer.press("Enter");
  await approveIfNeeded(page);
  await page.getByText("want me to proceed?", { exact: false }).waitFor();
  checks.push("multi_turn_first");
  await composer.fill("Second turn: continue from the saved progress.");
  await composer.press("Enter");
  await approveIfNeeded(page);
  await page.getByText("Second turn: continue from the saved progress.", { exact: true }).waitFor();
  checks.push("multi_turn_second");

  // Goal create, active status, steer/continuation, completed status, and clear via new session.
  await composer.fill("/goal active accessibility verification");
  await composer.press("Enter");
  await page.getByText("Goal active", { exact: true }).waitFor();
  await page.getByText("accessibility verification", { exact: false }).first().waitFor();
  checks.push("goal_set_active");
  await composer.fill("Prioritize keyboard and screen reader verification.");
  await composer.press("Enter");
  await page.getByRole("button", { name: "Steer active run with queued message" }).click();
  await page.getByText("Steering received", { exact: false }).waitFor();
  checks.push("goal_steer");
  await composer.fill("/new");
  await page.getByRole("button", { name: /\/new\b/ }).click();
  await page.getByText("New session", { exact: false }).first().waitFor();
  const goalComposer = page.getByLabel("Message Agent Desktop");
  await goalComposer.fill("/goal complete");
  await goalComposer.press("Enter");
  await page.getByText("Goal complete", { exact: true }).waitFor();
  checks.push("goal_complete");
  await goalComposer.fill("/new");
  await page.getByRole("button", { name: /\/new\b/ }).click();
  await page.getByText("New session", { exact: false }).first().waitFor();
  check(!(await page.locator("body").innerText()).includes("Goal complete"), "goal state leaked into new session");
  checks.push("goal_clear_new_session");

  // Reopen a session for all slash-command discovery and representative actions.
  await composer.fill("Slash command discovery session.");
  await composer.press("Enter");
  await approveIfNeeded(page);
  const freshComposer = page.getByLabel("Message Agent Desktop");
  for (const command of expectedSlashCommands) {
    await freshComposer.fill(`/${command}`);
    check(await page.getByRole("button", { name: new RegExp(`/${command}\\b`) }).count() > 0, `missing slash command /${command}`);
  }
  checks.push(`slash_discovery_${expectedSlashCommands.length}`);

  // Action commands: terminal, MCP, memory, and compact. Each is selected from the same UI autocomplete.
  await freshComposer.fill("/terminal");
  await page.getByRole("button", { name: /\/terminal\b/ }).click();
  await page.getByText("Terminal", { exact: true }).last().waitFor();
  await page.getByRole("button", { name: "Close terminal" }).click();
  await freshComposer.fill("/mcp");
  await page.getByRole("button", { name: /\/mcp\b/ }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Close" }).click();
  await freshComposer.fill("/memory");
  await page.getByRole("button", { name: /\/memory\b/ }).click();
  const memoryHide = page.getByRole("button", { name: "Hide memory" });
  if (await memoryHide.count() === 0) {
    const memoryShow = page.getByRole("button", { name: "Show memory" });
    check(await memoryShow.count() > 0, "/memory did not leave a memory control in the UI");
    await memoryShow.click();
  }
  await memoryHide.waitFor();
  await page.keyboard.press("Escape");
  await freshComposer.fill("/compact");
  await freshComposer.press("Enter");
  await page.keyboard.press("Escape");
  checks.push("slash_actions");

  // Prompt-style slash commands and side question.
  await freshComposer.fill("/btw What is the current verification status?");
  await freshComposer.press("Enter");
  await page.getByText("That's a side question", { exact: false }).waitFor();
  await page.keyboard.press("Escape");
  await freshComposer.fill("/sentry");
  await page.getByRole("button", { name: /\/sentry\b/ }).click();
  check((await freshComposer.inputValue()).includes("$sentry:sentry"), "/sentry did not expand to the bundled skill mention");
  checks.push("prompt_commands");

  // Artifact-producing run and responsive overflow check.
  await freshComposer.fill("Please produce an artifact for this GUI verification.");
  await freshComposer.press("Enter");
  await approveIfNeeded(page);
  await page.getByText("Artifact UX recommendations.md", { exact: true }).waitFor();
  checks.push("artifacts");
  await page.setViewportSize({ width: 375, height: 812 });
  const responsive = await page.evaluate(() => ({
    innerWidth: window.innerWidth,
    scrollWidth: document.documentElement.scrollWidth,
    scrollHeight: document.documentElement.scrollHeight,
  }));
  check(responsive.scrollWidth <= responsive.innerWidth, "full GUI flow overflows horizontally at 375px");
  const compactContext = page.getByRole("button", { name: /Context ·/ });
  await compactContext.waitFor({ state: "visible" });
  check(await compactContext.getAttribute("aria-expanded") === "false", "mobile checkout context should start collapsed");
  const notice = page.locator('[role="status"]').filter({ hasText: "Context compaction" });
  if (await notice.isVisible()) {
    const noticeBox = await notice.boundingBox();
    const composerBox = await page.getByLabel("Message Agent Desktop").boundingBox();
    check(Boolean(noticeBox && composerBox), "mobile notice or composer has no layout box");
    check(noticeBox.y + noticeBox.height <= composerBox.y, "mobile notice overlaps the composer");
  }
  checks.push("responsive_375");
  await page.screenshot({ path: path.join(outDir, "full-gui-final.png"), fullPage: true, animations: "disabled" });
  check(errors.length === 0, `browser errors:\n${errors.join("\n")}`);
  const receipt = {
    schema_version: 1,
    benchmark: "agent_desktop_full_gui_smoke",
    status: "passed",
    mode: "mock_provider_no_paid_calls",
    checks,
    slash_commands: expectedSlashCommands,
    responsive,
    browser_console_errors: errors,
    screenshot: path.join(outDir, "full-gui-final.png"),
  };
  await writeFile(path.join(outDir, "receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.log(JSON.stringify({ ...receipt, output_dir: outDir }));
} catch (error) {
  const receipt = {
    schema_version: 1,
    benchmark: "agent_desktop_full_gui_smoke",
    status: "failed",
    mode: "mock_provider_no_paid_calls",
    checks,
    browser_console_errors: errors,
    failure: String(error?.stack ?? error),
    body_text: page ? (await page.locator("body").innerText().catch(() => "")).slice(-5000) : "",
  };
  await writeFile(path.join(outDir, "receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.error(JSON.stringify({ ...receipt, output_dir: outDir }));
  process.exitCode = 1;
} finally {
  await browser?.close();
  dev.kill("SIGTERM");
}
