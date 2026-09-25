import { execFileSync, spawn } from "node:child_process";
import { createServer } from "node:net";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";

import { launch } from "./launch.mjs";
import { stopProcessTree } from "./process-tree.mjs";

const repoDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const fixtureEntry = path.join(repoDir, "harness", "fixtures", "specialist-product-entry.ts");
const productEntry = path.resolve(process.env.SPECIALIST_E2E_PRODUCT_ENTRY ?? fixtureEntry);
const composition = process.env.SPECIALIST_E2E_COMPOSITION ?? "foundation_fixture";
const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
const outDir = process.env.SPECIALIST_E2E_OUTPUT_DIR
  ? path.resolve(process.env.SPECIALIST_E2E_OUTPUT_DIR)
  : path.join(repoDir, "target", "specialist-matrix-smoke", `${stamp}-${process.pid}`);
const desktopViewport = { width: 1440, height: 1000 };
const mobileViewport = { width: 375, height: 812 };

const deepScan = process.env.SECURITY_E2E_WORKFLOW === "deep";
const specialists = [
  {
    kind: "security",
    label: "Security",
    starter: deepScan ? "Deep scan this repository" : "Investigate a security question",
    promptIncludes: deepScan ? "exploitable paths" : "security question",
    workflow: deepScan ? "security:security-deep" : "security:assistant",
    provider: "local",
    skill: deepScan ? "security:security-deep" : "security:assistant",
    tabs: [],
  },
];

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

function screenshotPath(kind, state) {
  return path.join(outDir, `${kind}-${state}.png`);
}

const port = await reservePort();
const url = `http://127.0.0.1:${port}/`;
const dev = spawn(
  "pnpm",
  ["--dir", "app", "dev", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
  {
    cwd: repoDir,
    env: {
      ...process.env,
      DESKTOP_PRODUCT_ENTRY: productEntry,
      VITE_FORCE_MOCK_BRIDGE: "1",
      VITE_PRODUCT_DEV_AUTH: "1",
    },
    detached: process.platform !== "win32",
    stdio: ["ignore", "pipe", "pipe"],
  },
);
let serverOutput = "";
dev.stdout.on("data", (chunk) => { serverOutput += chunk; });
dev.stderr.on("data", (chunk) => { serverOutput += chunk; });

async function waitForServer() {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (dev.exitCode != null) throw new Error(`Vite exited early\n${serverOutput}`);
    try {
      if ((await fetch(url)).ok) return;
    } catch {}
    await sleep(100);
  }
  throw new Error(`Vite did not start\n${serverOutput}`);
}

async function newPage(browser, viewport = desktopViewport) {
  const context = await browser.newContext({ viewport });
  await context.addInitScript(() => {
    const accountScope = "id:specialist-matrix-e2e";
    const encodedScope = encodeURIComponent(accountScope);
    localStorage.setItem("agent-desktop.dev-account", JSON.stringify({
      user: { id: "specialist-matrix-e2e", name: "Specialist Matrix E2E", method: "local" },
    }));
    localStorage.setItem(`agent-desktop:local-agent:${encodedScope}`, JSON.stringify({
      cwd: "/tmp/specialist-matrix-e2e",
      model: "local-model",
      reasoningEffort: "high",
    }));
    localStorage.setItem(`agent-desktop:project-context:${encodedScope}`, JSON.stringify({
      cwd: "/tmp/specialist-matrix-e2e",
    }));
  });
  const page = await context.newPage();
  page.on("pageerror", (error) => browserErrors.push(error.stack ?? error.message));
  page.on("console", (message) => {
    if (message.type() === "error") browserErrors.push(message.text());
  });
  page.on("requestfailed", (request) => failedRequests.push({
    method: request.method(),
    resourceType: request.resourceType(),
    url: request.url(),
    failure: request.failure()?.errorText ?? "unknown failure",
  }));
  page.on("response", (response) => {
    const request = response.request();
    if (response.status() >= 400 && ["fetch", "xhr"].includes(request.resourceType())) {
      failedRequests.push({
        method: request.method(),
        resourceType: request.resourceType(),
        url: response.url(),
        failure: `HTTP ${response.status()}`,
      });
    }
  });
  return { context, page };
}

async function installProbe(page) {
  await page.waitForFunction(() => Boolean(window.__agentDesktopProfiling?.getBridge), null, {
    timeout: 10_000,
  });
  await page.evaluate(async () => {
    const bridge = await window.__agentDesktopProfiling.getBridge();
    const originalOpen = bridge.openSession.bind(bridge);
    const originalPrompt = bridge.prompt.bind(bridge);
    const originalListSkills = bridge.listSkills.bind(bridge);
    window.__specialistMatrixProbe = {
      failNextOpen: false,
      openCalls: [],
      promptCalls: [],
      skillCatalogCalls: [],
      catalog: await bridge.listSpecialistCatalog(),
    };
    bridge.openSession = async (provider, config, request) => {
      const call = structuredClone({ provider, config, request });
      window.__specialistMatrixProbe.openCalls.push(call);
      if (window.__specialistMatrixProbe.failNextOpen) {
        window.__specialistMatrixProbe.failNextOpen = false;
        throw new Error("Simulated specialist session start failure.");
      }
      return originalOpen(provider, config, request);
    };
    bridge.prompt = async (sessionId, blocks, attachments) => {
      window.__specialistMatrixProbe.promptCalls.push(structuredClone({
        sessionId,
        blocks,
        attachmentCount: attachments.length,
      }));
      return originalPrompt(sessionId, blocks, attachments);
    };
    bridge.listSkills = async (...args) => {
      const result = await originalListSkills(...args);
      window.__specialistMatrixProbe.skillCatalogCalls.push({
        cwd: args[0],
        names: result.skills.map((skill) => skill.invocationName),
      });
      return result;
    };
  });
}

async function presentation(page, specialist) {
  return page.getByRole("region", { name: `${specialist.kind} specialist analysis` });
}

async function verifyExample(page, specialist) {
  await page.locator(`[data-qa="specialist-intro-${specialist.kind}-example"]`).click();
  const example = page.getByRole("region", { name: `${specialist.kind} example analysis` });
  await example.waitFor();
  for (const view of ["Evidence", "Run"]) {
    await example.getByRole("tab", { name: view, exact: true }).click();
    check(
      await example.getByRole("tab", { name: view, exact: true }).getAttribute("aria-selected") === "true",
      `${specialist.label} example did not select ${view}`,
    );
  }
  await page.locator(`[data-qa="specialist-intro-${specialist.kind}-start"]`).click();
}

const checks = [];
const results = {};
const browserErrors = [];
const failedRequests = [];
const sourceRevision = execFileSync("git", ["rev-parse", "HEAD"], { cwd: repoDir, encoding: "utf8" }).trim();
const sourceDirty = execFileSync("git", ["status", "--porcelain"], { cwd: repoDir, encoding: "utf8" }).trim().length > 0;
let browser;
let currentPage;

try {
  await mkdir(outDir, { recursive: true });
  await waitForServer();
  browser = await launch();

  for (const specialist of specialists) {
    const { context, page } = await newPage(browser);
    currentPage = page;
    await page.goto(`${url}?specialistPreview=paid`, { waitUntil: "domcontentloaded" });
    await installProbe(page);
    await page.getByRole("button", { name: "Specialist lenses", exact: true }).click();
    await page.locator(`[data-qa="specialist-nav-${specialist.kind}"]`).click();
    const workspace = page.locator(`[data-qa="specialist-workspace-${specialist.kind}"]`);
    await workspace.waitFor();
    await workspace.getByText("Access ready", { exact: true }).waitFor({ timeout: 10_000 });
    await page.locator(`[data-qa="specialist-welcome-${specialist.kind}"]`).waitFor();

    const catalogKinds = await page.evaluate(
      () => window.__specialistMatrixProbe.catalog.manifests.map((manifest) => manifest.kind),
    );
    check(JSON.stringify(catalogKinds) === JSON.stringify(["security"]), "Catalog kinds drifted from the product contract");
    checks.push(`${specialist.kind}_catalog_and_ready_state`);

    check(await page.getByLabel(`Show ${specialist.label} sidebar`).count() === 0, "Security still exposes Insights");
    check(await workspace.getByRole("combobox").count() === 0, "Security still requires an organization");
    await verifyExample(page, specialist);
    checks.push(`${specialist.kind}_example_without_insights`);

    await page.getByRole("button", { name: `Start ${specialist.label}: ${specialist.starter}` }).click();
    const composer = page.getByLabel("Message Clark Code");
    const submittedPrompt = await composer.inputValue();
    check(submittedPrompt.includes(specialist.promptIncludes), `${specialist.label} starter did not prefill its real prompt`);
    await page.evaluate(() => { window.__specialistMatrixProbe.failNextOpen = true; });
    await composer.press("Enter");
    await page.getByText("Simulated specialist session start failure.", { exact: false }).waitFor();
    check(await composer.inputValue() === submittedPrompt, `${specialist.label} lost its draft after session-start failure`);
    check(!(await composer.isDisabled()), `${specialist.label} stayed disabled after session-start failure`);
    await page.screenshot({ path: screenshotPath(specialist.kind, "start-failure"), animations: "disabled" });
    checks.push(`${specialist.kind}_failed_start_restores_draft`);

    await composer.press("Enter");
    await page.getByText(submittedPrompt, { exact: true }).waitFor({ timeout: 10_000 });
    await page.waitForFunction(() => Object.values(
      window.__agentDesktopProfiling.store.getState().snapshot.runs,
    ).some((run) => run.status === "running"));
    checks.push(`${specialist.kind}_optimistic_start_and_running_state`);
    const finalText = deepScan
      ? "The presentation is ready. Use the view tabs to move from the map to supporting evidence and the run lifecycle."
      : "I read src/main.rs. It defines a main that prints a greeting. Next I'd wire up argument parsing — want me to proceed?";
    if (deepScan) {
      const livePresentation = await presentation(page, specialist);
      await livePresentation.waitFor({ timeout: 10_000 });
      for (const view of ["Evidence", "Run"]) {
        await livePresentation.getByRole("tab", { name: view, exact: true }).click();
      }
    }
    await page.getByText(finalText, { exact: true }).waitFor({ timeout: 10_000 });
    await page.waitForFunction(() => Object.values(
      window.__agentDesktopProfiling.store.getState().snapshot.runs,
    ).every((run) => run.status === "done"));
    await page.screenshot({ path: screenshotPath(specialist.kind, "complete"), animations: "disabled" });

    const boundary = await page.evaluate(() => {
      const state = window.__agentDesktopProfiling.store.getState();
      const conversation = state.conversations.find((item) => item.id === state.session?.id);
      return {
        probe: structuredClone(window.__specialistMatrixProbe),
        session: state.session,
        conversation,
        runs: Object.values(state.snapshot.runs),
        timeline: structuredClone(state.snapshot.timeline),
        queued: state.queued.length,
      };
    });
    check(boundary.probe.openCalls.length === 2, `${specialist.label} did not cross exactly one failed and one successful open boundary`);
    check(boundary.probe.promptCalls.length === 1, `${specialist.label} did not cross exactly one prompt boundary`);
    const successfulOpen = boundary.probe.openCalls[1];
    check(successfulOpen.provider === specialist.provider, `${specialist.label} used the wrong provider boundary`);
    check(successfulOpen.request.kind === "new", `${specialist.label} did not allocate a new session`);
    check(boundary.conversation?.specialist?.kind === specialist.kind, `${specialist.label} metadata lost its specialist kind`);
    check(boundary.conversation?.specialist?.workflow === specialist.workflow, `${specialist.label} metadata lost its workflow`);
    check(!boundary.conversation?.specialist?.organizationId, "Security retained organization binding");
    check(boundary.runs.length === 1 && boundary.runs[0].status === "done", `${specialist.label} run did not settle exactly once`);
    if (deepScan) check(boundary.runs[0].outcome?.stop_reason === "specialist_presentation", `${specialist.label} run lost its typed terminal outcome`);
    check(boundary.timeline.filter((item) => item.item === "specialist_presentation").length === (deepScan ? 1 : 0), `${specialist.label} typed presentation duplicated or disappeared`);
    check(boundary.queued === 0, `${specialist.label} left queued work after completion`);
    const blocks = boundary.probe.promptCalls[0].blocks;
    const deliveredSkill = blocks.find((block) => block.type === "skill_reference")?.name ?? null;
    check(deliveredSkill === specialist.skill, `${specialist.label} delivered the wrong skill/runtime prompt contract`);
    check(blocks.find((block) => block.type === "text")?.text === submittedPrompt, `${specialist.label} changed the human prompt at the provider boundary`);
    check(boundary.probe.promptCalls[0].attachmentCount === 0, `${specialist.label} attached unexpected files`);
    check(!boundary.conversation.specialist.repositoryId, "Security retained repository registration");
    check(!successfulOpen.config.extra.cloud_advisor, "Security implicitly attached a cloud advisor");
    checks.push(`${specialist.kind}_provider_authority_projection_and_terminal_state`);

    await page.getByRole("button", { name: "New session", exact: true }).click();
    await page.getByRole("button", { name: "Specialist lenses", exact: true }).click();
    await page.locator(`[data-qa="specialist-nav-${specialist.kind}"]`).click();
    const row = page.locator(`[data-qa^="specialist-conversation-${specialist.kind}-"]`).first();
    await row.waitFor();
    await row.locator("button").first().click();
    if (deepScan) await (await presentation(page, specialist)).waitFor({ timeout: 10_000 });
    await page.getByText(finalText, { exact: true }).waitFor();
    const conversationSurface = page.locator(`[data-qa="specialist-conversation-${specialist.kind}"]`);
    check(await conversationSurface.getByText(submittedPrompt, { exact: true }).count() === 1, `${specialist.label} transcript did not survive detach and reattach exactly once`);
    checks.push(`${specialist.kind}_detach_and_reattach_continuity`);

    await page.setViewportSize(mobileViewport);
    if (specialist.tabs.length > 0) {
      const [firstTab] = specialist.tabs[0];
      await page.locator(`[data-qa="specialist-tab-${specialist.kind}-${firstTab}"]:visible`).click();
      await page.getByRole("region", { name: `${specialist.label} canvas` }).waitFor({ state: "visible" });
      await page.getByRole("button", { name: "Chat", exact: true }).click();
    }
    await page.getByLabel("Message Clark Code").waitFor({ state: "visible" });
    const overflow = await page.evaluate(() => ({
      viewport: window.innerWidth,
      document: document.documentElement.scrollWidth,
    }));
    check(overflow.document <= overflow.viewport + 1, `${specialist.label} overflowed the mobile viewport`);
    await page.screenshot({ path: screenshotPath(specialist.kind, "mobile-reopened"), animations: "disabled" });
    checks.push(`${specialist.kind}_mobile_chat_canvas_and_overflow`);
    results[specialist.kind] = {
      conversation_id: boundary.session.id,
      run_id: boundary.runs[0].id,
      workflow: specialist.workflow,
      provider: specialist.provider,
      skill: specialist.skill,
      timeline_items: boundary.timeline.length,
      screenshots: {
        canvas: specialist.tabs.length > 0
          ? screenshotPath(specialist.kind, "canvas-tabs")
          : null,
        failed_start: screenshotPath(specialist.kind, "start-failure"),
        complete: screenshotPath(specialist.kind, "complete"),
        mobile: screenshotPath(specialist.kind, "mobile-reopened"),
      },
    };
    await context.close();
  }

  const { context: freeContext, page: freePage } = await newPage(browser);
  currentPage = freePage;
  await freePage.goto(`${url}?specialistPreview=free`, { waitUntil: "domcontentloaded" });
  await freePage.getByRole("button", { name: "Specialist lenses", exact: true }).click();
  for (const specialist of specialists) {
    await freePage.locator(`[data-qa="specialist-nav-${specialist.kind}"]`).click();
    await freePage.locator(`[data-qa="specialist-gate-${specialist.kind}"]`).waitFor();
    check(await freePage.getByLabel("Message Clark Code").count() === 0, `${specialist.label} free gate still exposed a runnable composer`);
  }
  await freePage.screenshot({ path: path.join(outDir, "subscription-access-gates.png"), animations: "disabled" });
  await freePage.getByRole("button", { name: "New session", exact: true }).click();
  const specialistDisclosure = freePage.getByRole("button", { name: "Specialist lenses", exact: true });
  check(await specialistDisclosure.getAttribute("aria-expanded") === "false", "ordinary chat should collapse specialist navigation");
  check(await freePage.locator('[data-qa^="specialist-nav-"]').count() === 0, "collapsed specialist navigation should hide lens rows");
  await freePage.screenshot({ path: path.join(outDir, "ordinary-chat-sidebar.png"), animations: "disabled" });
  checks.push("ordinary_chat_collapses_specialist_navigation");
  await freeContext.close();
  checks.push("all_subscription_access_gates_block_dispatch");

  check(browserErrors.length === 0, `Browser console errors: ${browserErrors.join("\n")}`);
  check(failedRequests.length === 0, `Failed requests: ${JSON.stringify(failedRequests)}`);
  const receipt = {
    schema_version: 1,
    benchmark: "agent_desktop_specialist_matrix_e2e",
    status: "passed",
    mode: "browser_product_composition_mock_provider_no_paid_calls",
    composition,
    product_entry: productEntry,
    source_revision: sourceRevision,
    source_dirty: sourceDirty,
    catalog_kinds: ["security"],
    provider: { kind: "mock", model: null, paid_calls: 0 },
    viewports: { desktop: desktopViewport, mobile: mobileViewport },
    results,
    checks,
    browser_console_errors: browserErrors,
    failed_requests: failedRequests,
  };
  await writeFile(path.join(outDir, "receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.log(JSON.stringify({ ...receipt, output_dir: outDir }));
} catch (error) {
  const receipt = {
    schema_version: 1,
    benchmark: "agent_desktop_specialist_matrix_e2e",
    status: "failed",
    composition,
    product_entry: productEntry,
    source_revision: sourceRevision,
    source_dirty: sourceDirty,
    checks,
    results,
    browser_console_errors: browserErrors,
    failed_requests: failedRequests,
    failure: String(error?.stack ?? error),
    body_text: currentPage
      ? (await currentPage.locator("body").innerText().catch(() => "")).slice(-5_000)
      : "",
  };
  await mkdir(outDir, { recursive: true });
  await writeFile(path.join(outDir, "receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.error(JSON.stringify({ ...receipt, output_dir: outDir }));
  process.exitCode = 1;
} finally {
  await browser?.close();
  await stopProcessTree(dev);
}
