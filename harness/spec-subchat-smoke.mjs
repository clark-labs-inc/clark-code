import { spawn, execFileSync } from "node:child_process";
import { once } from "node:events";
import { createServer } from "node:net";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";

import { launch } from "./launch.mjs";

const repoDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const productEntry = path.join(repoDir, "harness", "fixtures", "spec-product-entry.ts");
const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
const outDir = process.env.SPEC_SUBCHAT_E2E_OUTPUT_DIR
  ? path.resolve(process.env.SPEC_SUBCHAT_E2E_OUTPUT_DIR)
  : path.join(repoDir, "target", "spec-subchat-smoke", `${stamp}-${process.pid}`);
const productProbeKey = "agent-desktop:spec-subchat-product-probe";
const productControlKey = "agent-desktop:spec-subchat-product-control";

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

async function stopDevServer(child) {
  if (!child || child.exitCode !== null) return;
  try {
    if (process.platform === "win32") {
      execFileSync("taskkill", ["/pid", String(child.pid), "/t", "/f"], {
        stdio: "ignore",
      });
    } else if (child.pid) {
      // pnpm launches Vite as a child. Kill the detached process group so a
      // failed browser assertion cannot leave Vite holding the CI step open.
      process.kill(-child.pid, "SIGTERM");
    }
  } catch {
    // The child may have exited between the status check and the tree kill.
  }
  await Promise.race([
    once(child, "exit"),
    sleep(5_000),
  ]);
  if (child.exitCode === null) {
    try {
      if (process.platform === "win32") {
        execFileSync("taskkill", ["/pid", String(child.pid), "/t", "/f"], {
          stdio: "ignore",
        });
      } else if (child.pid) {
        process.kill(-child.pid, "SIGKILL");
      }
    } catch {
      // The process was already reaped.
    }
  }
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
  for (let attempt = 0; attempt < 80; attempt += 1) {
    if (dev.exitCode != null) throw new Error(`Vite exited early\n${serverOutput}`);
    try {
      if ((await fetch(url)).ok) return;
    } catch {}
    await sleep(100);
  }
  throw new Error(`Vite did not start\n${serverOutput}`);
}

const sourceRevision = execFileSync("git", ["rev-parse", "HEAD"], {
  cwd: repoDir,
  encoding: "utf8",
}).trim();
const sourceDirty = execFileSync("git", ["status", "--porcelain"], {
  cwd: repoDir,
  encoding: "utf8",
}).trim().length > 0;
const desktopViewport = { width: 1440, height: 1000 };
const mobileViewport = { width: 375, height: 812 };
const initialPrompt = "Create a concise specification for customer segmentation.";
const selectedText = "This spec defines how the system segments customers based on behavior, demographics, and engagement signals to support targeted experiences.";
const scopedQuestion = "Make the executive summary clearer about why a visitor would pay.";
const recoveredQuestion = "Explain the customer benefit in one direct sentence.";
const directQuestion = "Make the target user explicit in this overview.";
const firstReply = "I updated the living Customer Segmentation specification.";
const checks = [];
const browserErrors = [];
const failedRequests = [];
const screenshots = {
  document: path.join(outDir, "01-document-ready.png"),
  catalogFailure: path.join(outDir, "02-catalog-failure.png"),
  catalogUnavailable: path.join(outDir, "03-catalog-unavailable.png"),
  documentFailure: path.join(outDir, "04-document-failure.png"),
  sendRejected: path.join(outDir, "05-send-rejected.png"),
  starting: path.join(outDir, "06-subchat-starting.png"),
  updating: path.join(outDir, "07-subchat-updating.png"),
  complete: path.join(outDir, "08-subchat-complete.png"),
  mobile: path.join(outDir, "09-subchat-reopened-mobile.png"),
};

let browser;
let page;
try {
  await mkdir(outDir, { recursive: true });
  await waitForServer();
  browser = await launch();
  const context = await browser.newContext({ viewport: desktopViewport });
  await context.addInitScript(() => {
    const accountScope = "id:spec-subchat-e2e";
    const encodedScope = encodeURIComponent(accountScope);
    localStorage.setItem("agent-desktop.dev-account", JSON.stringify({
      user: { id: "spec-subchat-e2e", name: "Spec Subchat E2E", method: "local" },
    }));
    localStorage.setItem(`agent-desktop:local-agent:${encodedScope}`, JSON.stringify({
      cwd: "/tmp/spec-subchat-e2e",
      model: "local-model",
      reasoningEffort: "high",
    }));
    localStorage.setItem(`agent-desktop:project-context:${encodedScope}`, JSON.stringify({
      cwd: "/tmp/spec-subchat-e2e",
    }));
  });

  page = await context.newPage();
  page.on("pageerror", (error) => browserErrors.push(error.stack ?? error.message));
  page.on("console", (message) => {
    if (message.type() === "error") browserErrors.push(message.text());
  });
  page.on("requestfailed", (request) => {
    failedRequests.push({
      method: request.method(),
      resourceType: request.resourceType(),
      url: request.url(),
      failure: request.failure()?.errorText ?? "unknown failure",
    });
  });
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

  await page.goto(url, { waitUntil: "domcontentloaded" });
  await page.getByRole("button", { name: "Spec", exact: true }).click();
  const mainComposer = page.getByLabel("Message Clark Code");
  await mainComposer.waitFor({ state: "visible" });
  await mainComposer.fill(initialPrompt);
  await mainComposer.press("Enter");
  await page.getByRole("heading", { name: "Customer Segmentation", exact: true })
    .waitFor({ timeout: 10_000 });
  await page.getByText("Saved locally", { exact: true }).waitFor();
  await page.getByText(selectedText, { exact: true }).waitFor();
  await page.screenshot({ path: screenshots.document, animations: "disabled" });
  checks.push("whole_spec_created_through_real_composer");

  await page.getByText(selectedText, { exact: true }).click();
  const thread = page.getByRole("complementary", {
    name: "Discuss selected specification content",
  });
  await thread.waitFor({ state: "visible" });
  check(await thread.getByRole("button", { name: "Ask about this" }).isVisible(), "Ask action is missing");
  check(await thread.getByRole("button", { name: "Suggest edit" }).isVisible(), "Edit action is missing");
  const selectionComposer = thread.getByLabel("Selection discussion message");
  const selectionSend = thread.getByRole("button", { name: "Send selection discussion" });
  await selectionComposer.fill("");
  check(await selectionSend.isDisabled(), "Empty scoped draft did not disable send");
  checks.push("idle_empty_draft_disables_send");
  await thread.getByRole("button", { name: "Suggest edit" }).click();
  check(
    (await selectionComposer.inputValue()).includes("Make this clearer and more specific"),
    "Suggested section action did not reach the scoped composer",
  );
  checks.push("document_selection_opened_and_action_prefilled_draft");

  await page.evaluate(() => {
    const store = window.__agentDesktopProfiling.store;
    const state = store.getState();
    store.setState({ snapshot: { ...state.snapshot, starting: true } });
  });
  check(await selectionSend.isDisabled(), "Active work did not disable scoped send");
  check(!(await selectionComposer.isDisabled()), "Busy state incorrectly disabled draft editing");
  await page.evaluate(() => {
    const store = window.__agentDesktopProfiling.store;
    const state = store.getState();
    store.setState({ snapshot: { ...state.snapshot, starting: false } });
  });
  await selectionSend.waitFor({ state: "visible" });
  check(!(await selectionSend.isDisabled()), "Scoped send did not recover after busy state cleared");
  checks.push("busy_blocks_send_but_preserves_editable_draft");

  await page.evaluate(async () => {
    const bridge = await window.__agentDesktopProfiling.getBridge();
    const originalList = bridge.listSkills.bind(bridge);
    const originalPrompt = bridge.prompt.bind(bridge);
    const fullCatalog = await originalList("/tmp/spec-subchat-e2e", null);
    window.__specSubchatE2EProbe = {
      mode: "current",
      listCalls: [],
      reloadCalls: [],
      promptCalls: [],
    };
    bridge.listSkills = async (...args) => {
      const mode = window.__specSubchatE2EProbe.mode;
      window.__specSubchatE2EProbe.listCalls.push(mode);
      await new Promise((resolve) => setTimeout(resolve, 150));
      if (mode === "catalog_failure" || mode === "list_failure_recovery") {
        throw new Error("catalog list failed");
      }
      if (mode === "catalog_unavailable" || mode === "stale_recovery") {
        return {
          ...fullCatalog,
          projectRoot: args[0],
          skills: fullCatalog.skills.filter((skill) => skill.invocationName !== "spec:spec"),
        };
      }
      return {
        ...fullCatalog,
        projectRoot: args[0],
      };
    };
    bridge.reloadSkills = async (...args) => {
      const mode = window.__specSubchatE2EProbe.mode;
      window.__specSubchatE2EProbe.reloadCalls.push(mode);
      await new Promise((resolve) => setTimeout(resolve, 150));
      if (mode === "catalog_failure") throw new Error("catalog reload failed");
      if (mode === "catalog_unavailable") {
        return { ...fullCatalog, projectRoot: args[0], skills: [] };
      }
      return { ...fullCatalog, projectRoot: args[0] };
    };
    bridge.prompt = async (sessionId, blocks, attachments) => {
      window.__specSubchatE2EProbe.promptCalls.push({
        sessionId,
        blocks: structuredClone(blocks),
        attachmentCount: attachments.length,
      });
      return originalPrompt(sessionId, blocks, attachments);
    };
  });

  const setProbeMode = async (mode) => page.evaluate((nextMode) => {
    window.__specSubchatE2EProbe.mode = nextMode;
  }, mode);
  const promptCallCount = async () => page.evaluate(
    () => window.__specSubchatE2EProbe.promptCalls.length,
  );
  const dismissFeedback = async () => page.evaluate(() => {
    const state = window.__agentDesktopProfiling.store.getState();
    state.dismissNotice();
    state.dismissWarning();
  }).then(async () => {
    // Sonner dismisses the fixed warning toast id asynchronously. Wait for
    // that DOM removal before the next rejection probe, otherwise a rapid
    // warning replacement can be swallowed by the closing toast instance.
    await page.waitForFunction(
      () => !document.querySelector('[data-sonner-toast][data-type="warning"]'),
      undefined,
      { timeout: 5_000 },
    );
  });
  const expectRejectedTransition = async ({
    mode,
    question,
    notice,
    screenshot,
    toastType = "warning",
    configure,
    reset,
    control,
  }) => {
    await setProbeMode(mode);
    await configure?.();
    if (control) {
      check(
        await page.evaluate((key) => localStorage.getItem(key), productControlKey)
          === JSON.stringify(control),
        "Spec product preparation control was not installed before the rejection probe",
      );
    }
    const promptsBefore = await promptCallCount();
    await selectionComposer.fill(question);
    await selectionSend.click();
    await thread.getByText("Starting this discussion…", { exact: true }).waitFor();
    await page.getByText(notice, { exact: true }).waitFor();
    await page.locator(`[data-sonner-toast][data-type="${toastType}"]`).filter({ hasText: notice }).waitFor();
    check((await selectionComposer.inputValue()) === question, `${mode} did not restore the scoped draft`);
    check(!(await selectionComposer.isDisabled()), `${mode} left the scoped composer disabled`);
    check((await promptCallCount()) === promptsBefore, `${mode} unexpectedly crossed the provider boundary`);
    await page.screenshot({ path: screenshot, animations: "disabled" });
    await reset?.();
    await dismissFeedback();
  };

  await expectRejectedTransition({
    mode: "catalog_failure",
    question: "Preserve this draft when the workflow catalog fails.",
    notice: "Could not load the Spec workflow: Error: catalog reload failed",
    screenshot: screenshots.catalogFailure,
  });
  checks.push("catalog_failure_reports_error_and_restores_draft");

  await expectRejectedTransition({
    mode: "catalog_unavailable",
    question: "Preserve this draft when the Spec workflow is unavailable.",
    notice: "The Spec workflow is unavailable. Reload skills and try again.",
    screenshot: screenshots.catalogUnavailable,
  });
  checks.push("catalog_unavailable_reports_error_and_restores_draft");

  await expectRejectedTransition({
    mode: "current",
    question: "Preserve this draft when the saved document cannot load.",
    notice: "Could not load the saved spec. Try again.",
    screenshot: screenshots.documentFailure,
    control: { prepareDocument: "fail" },
    configure: () => page.evaluate((key) => {
      localStorage.setItem(key, JSON.stringify({ prepareDocument: "fail" }));
    }, productControlKey),
    reset: () => page.evaluate((key) => localStorage.removeItem(key), productControlKey),
  });
  checks.push("document_preparation_failure_reports_error_and_restores_draft");

  await expectRejectedTransition({
    mode: "current",
    question: "Preserve this draft when the canonical send rejects it.",
    notice: "Clark Code is finishing active work before updating; send after it relaunches.",
    screenshot: screenshots.sendRejected,
    toastType: "success",
    configure: () => page.evaluate(() => {
      window.__agentDesktopProfiling.store.setState({ updateWaiting: true });
    }),
    reset: () => page.evaluate(() => {
      window.__agentDesktopProfiling.store.setState({ updateWaiting: false });
    }),
  });
  checks.push("not_sent_restores_draft_without_provider_dispatch");

  await setProbeMode("stale_recovery");
  await selectionComposer.fill(scopedQuestion);
  await selectionSend.click();
  await selectionSend.evaluate((button) => button.click());
  await thread.getByText("Starting this discussion…", { exact: true }).waitFor();
  check((await selectionComposer.inputValue()) === "", "Scoped composer did not clear atomically");
  await page.screenshot({ path: screenshots.starting, animations: "disabled" });
  checks.push("visible_preflight_atomic_clear_and_duplicate_suppression");

  await thread.getByText(scopedQuestion, { exact: true }).waitFor({ timeout: 5_000 });
  await thread.getByText("Updating this section…", { exact: true }).waitFor();
  check(await selectionSend.isDisabled(), "Active scoped run did not disable another send");
  await page.screenshot({ path: screenshots.updating, animations: "disabled" });
  checks.push("projected_turn_moves_from_starting_to_updating");
  await thread.getByText(firstReply, { exact: true }).waitFor({ timeout: 10_000 });
  checks.push("stale_catalog_reloaded_and_scoped_prompt_completed");

  await setProbeMode("list_failure_recovery");
  await selectionComposer.fill(recoveredQuestion);
  await selectionSend.click();
  await thread.getByText(recoveredQuestion, { exact: true }).waitFor();
  await thread.getByText(firstReply, { exact: true }).nth(1).waitFor({ timeout: 10_000 });
  checks.push("failed_catalog_read_recovered_through_reload");

  await setProbeMode("current");
  await selectionComposer.fill(directQuestion);
  await selectionSend.click();
  await thread.getByText(directQuestion, { exact: true }).waitFor();
  await thread.getByText(firstReply, { exact: true }).nth(2).waitFor({ timeout: 10_000 });
  await page.screenshot({ path: screenshots.complete, animations: "disabled" });
  checks.push("current_catalog_dispatches_without_reload_and_settles");

  const boundary = await page.evaluate((probeKey) => {
    const state = window.__agentDesktopProfiling.store.getState();
    const messages = state.snapshot.timeline
      .filter((item) => item.item === "message")
      .map((item) => ({
        run: item.run,
        role: item.role,
        text: item.blocks
          .filter((block) => block.type === "text")
          .map((block) => block.text)
          .join("\n"),
      }));
    return {
      bridge: window.__specSubchatE2EProbe,
      productPreparations: JSON.parse(localStorage.getItem(probeKey) ?? "[]"),
      sessionId: state.session?.id ?? null,
      runs: Object.values(state.snapshot.runs).map((run) => ({
        id: run.id,
        status: run.status,
      })),
      messages,
      artifacts: state.snapshot.artifacts.map((artifact) => ({
        id: artifact.id,
        title: artifact.title,
        mimeType: artifact.mime_type,
      })),
      queued: state.queued.length,
    };
  }, productProbeKey);
  check(boundary.bridge.listCalls.length === 7, "Scoped transitions did not perform the expected catalog reads");
  check(boundary.bridge.reloadCalls.length === 4, "Scoped transitions did not perform the expected recovery reloads");
  check(boundary.bridge.promptCalls.length === 3, "Successful scoped transitions did not dispatch exactly three prompts");
  check(
    boundary.bridge.promptCalls.every((call) => call.attachmentCount === 0),
    "Scoped text transitions unexpectedly attached files",
  );
  for (const [index, comment] of [scopedQuestion, recoveredQuestion, directQuestion].entries()) {
    const providerBlocks = boundary.bridge.promptCalls[index].blocks;
    const providerText = providerBlocks.find((block) => block.type === "text")?.text ?? "";
    const providerSkill = providerBlocks.find((block) => block.type === "skill_reference");
    check(providerSkill?.name === "spec:spec", `Spec skill reference ${index + 1} did not reach the provider boundary`);
    check(providerText.includes(`<selected_spec_content>\n${selectedText}\n</selected_spec_content>`), `Selected excerpt ${index + 1} was not preserved at the provider boundary`);
    check(providerText.includes(`<scoped_comment>\n${comment}\n</scoped_comment>`), `Scoped comment ${index + 1} was not preserved at the provider boundary`);
    check(providerText.includes("<spec_document>"), `Prepared document authority ${index + 1} was not included`);
  }
  check(boundary.productPreparations.length === 5, "Product document preparation count did not match accepted transition boundaries");
  check(
    boundary.productPreparations.every(
      (preparation) => preparation.filename === boundary.productPreparations[0].filename,
    ),
    "A scoped transition changed the canonical Spec filename",
  );
  check(boundary.runs.length === 4 && boundary.runs.every((run) => run.status === "done"), "Canonical runs did not settle exactly once");
  check(boundary.messages.filter((message) => message.role === "user").length === 4, "Canonical user turns duplicated or disappeared");
  check(boundary.messages.filter((message) => message.role === "agent").length === 4, "Canonical agent turns duplicated or disappeared");
  check(boundary.artifacts.length === 1 && boundary.artifacts[0].mimeType === "text/markdown", "Living Spec artifact was not retained");
  check(boundary.queued === 0, "Scoped prompt remained queued after terminal completion");
  const publicText = await page.locator("body").innerText();
  check(!publicText.includes("<selected_spec_content>"), "Scoped prompt envelope leaked into the UI");
  check(!publicText.includes("spec:spec"), "Internal Spec skill name leaked into the UI");
  checks.push("provider_product_and_canonical_boundaries_verified");

  await thread.getByRole("button", { name: "Close selection discussion" }).click();
  const otherSection = "Teams lack a consistent, automated way to group customers. Manual exports and spreadsheet logic are slow, error-prone, and difficult to maintain.";
  await page.getByText(otherSection, { exact: true }).click();
  await thread.waitFor({ state: "visible" });
  check(!(await thread.innerText()).includes(scopedQuestion), "Scoped conversation leaked into another section");
  await thread.getByRole("button", { name: "Close selection discussion" }).click();
  await page.getByText(selectedText, { exact: true }).click();
  await thread.getByText(scopedQuestion, { exact: true }).waitFor();
  await thread.getByText(recoveredQuestion, { exact: true }).waitFor();
  await thread.getByText(directQuestion, { exact: true }).waitFor();
  check(await thread.getByText(firstReply, { exact: true }).count() === 3, "Settled scoped replies did not persist after reopen");
  checks.push("section_isolation_and_close_reopen_continuity");

  await page.setViewportSize(mobileViewport);
  const threadBox = await thread.boundingBox();
  check(Boolean(threadBox), "Mobile subchat has no layout box");
  check(threadBox.x >= 0 && threadBox.y >= 0, "Mobile subchat starts outside the viewport");
  check(threadBox.x + threadBox.width <= mobileViewport.width, "Mobile subchat is clipped horizontally");
  check(threadBox.y + threadBox.height <= mobileViewport.height, "Mobile subchat is clipped vertically");
  check(await thread.getByRole("button", { name: "Send selection discussion" }).isVisible(), "Mobile send control is unreachable");
  const mobileLayout = await page.evaluate(() => ({
    innerWidth: window.innerWidth,
    scrollWidth: document.documentElement.scrollWidth,
  }));
  check(mobileLayout.scrollWidth <= mobileLayout.innerWidth, "Mobile Spec workspace overflows horizontally");
  await page.screenshot({ path: screenshots.mobile, animations: "disabled" });
  checks.push("mobile_reopened_thread_remains_reachable");

  check(browserErrors.length === 0, `Browser errors:\n${browserErrors.join("\n")}`);
  check(failedRequests.length === 0, `Failed browser requests:\n${JSON.stringify(failedRequests, null, 2)}`);
  checks.push("browser_console_and_network_clean");

  const receipt = {
    schema_version: 1,
    benchmark: "agent_desktop_spec_subchat_e2e",
    status: "passed",
    mode: "browser_product_composition_mock_provider_no_paid_calls",
    source_revision: sourceRevision,
    source_dirty: sourceDirty,
    url,
    viewports: { desktop: desktopViewport, mobile: mobileViewport },
    provider: { kind: "mock", model: null, paid_calls: 0 },
    conversation_id: boundary.sessionId,
    run_ids: boundary.runs.map((run) => run.id),
    canonical: {
      runs: boundary.runs,
      message_count: boundary.messages.length,
      artifact_count: boundary.artifacts.length,
      queued_count: boundary.queued,
    },
    checks,
    browser_console_errors: browserErrors,
    failed_requests: failedRequests,
    screenshots,
  };
  await writeFile(path.join(outDir, "receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.log(JSON.stringify({ ...receipt, output_dir: outDir }));
} catch (error) {
  const receipt = {
    schema_version: 1,
    benchmark: "agent_desktop_spec_subchat_e2e",
    status: "failed",
    mode: "browser_product_composition_mock_provider_no_paid_calls",
    source_revision: sourceRevision,
    source_dirty: sourceDirty,
    checks,
    browser_console_errors: browserErrors,
    failed_requests: failedRequests,
    failure: String(error?.stack ?? error),
    body_text: page
      ? (await page.locator("body").innerText().catch(() => "")).slice(-5_000)
      : "",
  };
  await mkdir(outDir, { recursive: true });
  await writeFile(path.join(outDir, "receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.error(JSON.stringify({ ...receipt, output_dir: outDir }));
  process.exitCode = 1;
} finally {
  await browser?.close();
  await stopDevServer(dev);
}
