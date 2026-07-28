import { chromium } from "../../harness/node_modules/playwright/index.mjs";
import { mkdir, rename } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const PUBLIC = join(HERE, "public");
const TAKE = join(PUBLIC, "clark-ui-flagship.webm");
const VIEWPORT = { width: 1600, height: 1000 };
const CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

await mkdir(PUBLIC, { recursive: true });

const browser = await chromium.launch({ headless: true, executablePath: CHROME });
const context = await browser.newContext({
  viewport: VIEWPORT,
  recordVideo: { dir: PUBLIC, size: VIEWPORT },
  deviceScaleFactor: 1,
});
const page = await context.newPage();

await page.addInitScript(() => {
  localStorage.setItem("clark.theme", "dark");
  localStorage.setItem("clark-desktop:permission-mode", "ask");
  localStorage.setItem(
    "clark.auth.session",
    JSON.stringify({
      user: {
        id: "promo-user",
        name: "Alex Chen",
        email: "alex@northstar.dev",
        method: "local",
      },
      clark: { endpoint: "ws://localhost:8400/ws" },
    }),
  );
});
await page.goto("http://localhost:1420", { waitUntil: "networkidle" });
await page.waitForFunction(() => Boolean(window.__clarkStore));

await page.evaluate(() => {
  const cursor = document.createElement("div");
  cursor.id = "promo-cursor";
  Object.assign(cursor.style, {
    position: "fixed",
    zIndex: "999999",
    width: "18px",
    height: "18px",
    borderRadius: "999px",
    border: "2px solid rgba(255,255,255,.95)",
    background: "rgba(124,92,255,.35)",
    boxShadow: "0 0 0 6px rgba(124,92,255,.12), 0 3px 12px rgba(0,0,0,.45)",
    pointerEvents: "none",
    transform: "translate(-50%, -50%)",
    transition: "left 90ms linear, top 90ms linear, transform 120ms ease",
    left: "800px",
    top: "880px",
  });
  document.body.appendChild(cursor);
  document.addEventListener("mousemove", (event) => {
    cursor.style.left = `${event.clientX}px`;
    cursor.style.top = `${event.clientY}px`;
  });
  document.addEventListener("mousedown", () => {
    cursor.style.transform = "translate(-50%, -50%) scale(.72)";
  });
  document.addEventListener("mouseup", () => {
    cursor.style.transform = "translate(-50%, -50%) scale(1)";
  });
});

const capabilities = {
  streaming: true,
  permissions: true,
  fs: true,
  terminal: true,
  load_session: false,
  modes: [],
  collaboration_modes: ["default", "plan"],
};

const run = "flagship-run";
const base = {
  session: "flagship-session",
  runs: {
    [run]: {
      id: run,
      status: "running",
      checkpoint: "before-reconnect-fix",
    },
  },
  timeline: [],
  tool_calls: {},
  artifacts: [],
  provider_incidents: {},
};

await page.evaluate(
  ({ capabilities, base }) => {
    const store = window.__clarkStore;
    const current = store.getState();
    store.setState({
      providers: [{ id: "local", label: "Clark Code", capabilities }],
      activeProvider: "local",
      session: {
        id: "flagship-session",
        provider: "local",
        capabilities,
        collaboration_mode: "default",
        environment: {
          checkout_root: "/Users/demo/northstar-desktop",
          repository_root: "/Users/demo/northstar-desktop",
          workspace_roots: ["/Users/demo/northstar-desktop"],
          remote: false,
        },
      },
      conversations: [
        {
          id: "flagship-session",
          title: "Fix the reconnect regression",
          provider: "local",
          project: "/Users/demo/northstar-desktop",
          createdAt: Date.now() - 120_000,
          updatedAt: Date.now(),
        },
      ],
      activeProjectRoot: "/Users/demo/northstar-desktop",
      localSettings: {
        ...current.localSettings,
        cwd: "/Users/demo/northstar-desktop",
      },
      snapshot: base,
      error: null,
      warning: null,
      notice: null,
      sidebarCollapsed: false,
    });
  },
  { capabilities, base },
);

const push = async (snapshot) => {
  await page.evaluate(async (next) => {
    window.__clarkStore.setState({ snapshot: next });
    const fanOut = await import("/src/store/fanOutStore.ts");
    fanOut.syncFanOut(next.fan_out);
  }, structuredClone(snapshot));
};

const pause = (ms) => page.waitForTimeout(ms);
const snap = structuredClone(base);

await pause(800);
const composer = page.getByLabel("Message Clark");
const prompt =
  "Customer says reconnect spins forever after sleep on Windows. Logs attached. Find the root cause—don’t edit until you can reproduce it locally and on our runner.";
await page.mouse.move(790, 880, { steps: 18 });
await composer.click();
await composer.pressSequentially(prompt, { delay: 18 });
await pause(700);
await page.mouse.move(1538, 948, { steps: 12 });
await composer.press("Meta+A");
await composer.press("Backspace");

snap.timeline.push({
  item: "message",
  run,
  role: "user",
  blocks: [{ type: "text", text: prompt }],
});
snap.execution_checklist = {
  revision: 1,
  steps: [
    { title: "Reproduce from the reconnect log", status: "in_progress" },
    { title: "Trace cursor persistence across platforms", status: "pending" },
    { title: "Verify locally and on the runner", status: "pending" },
  ],
};
snap.timeline.push({
  item: "execution_checklist",
  run,
  checklist: structuredClone(snap.execution_checklist),
});
await push(snap);
await pause(1700);

snap.timeline.push({
  item: "message",
  run,
  role: "agent",
  phase: "commentary",
  blocks: [
    {
      type: "text",
      text: "I’m reconstructing the reconnect boundary first. I’ll keep the current branch untouched until the failure is reproduced.",
    },
  ],
});
snap.tool_calls["read-log"] = {
  id: "read-log",
  title: "Read customer-reconnect.log",
  kind: "read",
  status: "completed",
  locations: [{ path: "fixtures/customer-reconnect.log", line: 184 }],
  content: [
    {
      type: "text",
      text:
        "08:41:17 resume cursor=evt_932\\r\\n\n" +
        "08:41:17 rejected cursor: invalid identifier\n" +
        "08:41:18 reconnect attempt=7 delay=1000ms",
    },
  ],
};
snap.timeline.push({ item: "tool_call", id: "read-log", run });
await push(snap);
await pause(1200);

const now = Date.now();
snap.fan_out = {
  title: "Reproduce, trace, and check the upstream resume contract",
  total: 3,
  done: 0,
  running: 3,
  agents: [
    {
      id: "reproduce",
      label: "Reproduce the Windows fixture",
      status: "running",
      objective: "Turn the customer log into the smallest deterministic failure.",
      activity: "Running the saved reconnect transcript",
      attempt: 1,
      started_at_ms: now - 12_000,
      updated_at_ms: now,
    },
    {
      id: "trace",
      label: "Trace cursor persistence",
      status: "running",
      objective: "Follow the resume cursor from disk through reconnect validation.",
      activity: "Inspecting the cursor codec and retry loop",
      attempt: 1,
      started_at_ms: now - 9_000,
      updated_at_ms: now,
    },
    {
      id: "contract",
      label: "Check the upstream contract",
      status: "running",
      objective: "Verify the exact event ID grammar and line-ending rules.",
      activity: "Comparing protocol docs and release notes",
      attempt: 1,
      started_at_ms: now - 7_000,
      updated_at_ms: now,
    },
  ],
};
await push(snap);
await pause(900);

const firstAgent = page.getByRole("button", {
  name: /Reproduce the Windows fixture.*Open subagent details/,
});
if (await firstAgent.count()) {
  const box = await firstAgent.boundingBox();
  if (box) {
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 18 });
  }
  await firstAgent.click();
  await pause(1800);
  const close = page.getByLabel("Close subagents");
  if (await close.count()) {
    const box = await close.boundingBox();
    if (box) await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 14 });
    await close.click();
  }
}
await pause(800);

snap.tool_calls["first-test"] = {
  id: "first-test",
  title: "Run reconnect fixture — failed",
  kind: "execute",
  status: "failed",
  locations: [],
  content: [
    {
      type: "text",
      text:
        "assertion failed: resume cursor\n" +
        "  expected: \"evt_932\"\n" +
        "  received: \"evt_932\\r\"\n" +
        "test reconnects_from_windows_receipt ... FAILED",
    },
  ],
};
snap.timeline.push({ item: "tool_call", id: "first-test", run });
snap.timeline.push({
  item: "message",
  run,
  role: "agent",
  phase: "commentary",
  blocks: [
    {
      type: "text",
      text: "The timeout theory was wrong. The failure survives with retries disabled: Windows leaves a carriage return on the persisted event ID.",
    },
  ],
});
snap.execution_checklist = {
  revision: 2,
  steps: [
    { title: "Reproduce from the reconnect log", status: "completed" },
    { title: "Trace cursor persistence across platforms", status: "in_progress" },
    { title: "Verify locally and on the runner", status: "pending" },
  ],
};
const checklist = snap.timeline.find((item) => item.item === "execution_checklist");
checklist.checklist = structuredClone(snap.execution_checklist);
snap.fan_out = {
  ...snap.fan_out,
  done: 2,
  running: 1,
  agents: snap.fan_out.agents.map((agent) =>
    agent.id === "contract"
      ? {
          ...agent,
          status: "running",
          activity: "Confirming event ID line-ending rules",
          updated_at_ms: Date.now(),
        }
      : {
          ...agent,
          status: "done",
          activity: "Complete",
          result:
            agent.id === "reproduce"
              ? "Reproduced: the persisted cursor is evt_932\\r after LF-only trimming."
              : "The retry loop is healthy; cursor normalization is the first broken boundary.",
          updated_at_ms: Date.now(),
        },
  ),
};
await push(snap);
await pause(500);
const failedTest = page
  .locator('button[aria-expanded="false"]')
  .filter({ hasText: /Run reconnect fixture/ })
  .last();
if (await failedTest.count()) {
  const box = await failedTest.boundingBox();
  if (box) await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 14 });
  await failedTest.click();
}
await pause(2200);

snap.tool_calls["research"] = {
  id: "research",
  title: "clark_research: resume cursor line endings",
  kind: "research",
  status: "in_progress",
  locations: [],
  raw_input: { query: "resume cursor event ID line endings Windows CRLF" },
  content: [],
};
snap.timeline.push({ item: "tool_call", id: "research", run });
await push(snap);
await pause(1800);

snap.tool_calls.research = {
  ...snap.tool_calls.research,
  status: "completed",
  content: [
    {
      type: "text",
      text:
        "**The resume identifier is an opaque token, not a line.** Readers must remove transport line endings before validation.\n\n" +
        "- Windows receipts commonly terminate records with `\\r\\n`.\n" +
        "- Removing only `\\n` leaves `\\r` inside the identifier.\n" +
        "- Normalize the record boundary; do not mutate the token itself.\n\n" +
        "The protocol contract and two upstream implementations agree on this boundary.",
    },
  ],
};
snap.fan_out = {
  ...snap.fan_out,
  done: 3,
  running: 0,
  agents: snap.fan_out.agents.map((agent) => ({
    ...agent,
    status: "done",
    activity: "Complete",
    result:
      agent.result ??
      "Confirmed: transport line endings must be removed before the opaque cursor is validated.",
    updated_at_ms: Date.now(),
  })),
};
await push(snap);
await pause(1800);

snap.tool_calls.edit = {
  id: "edit",
  title: "Edit src/runtime/cursor.rs",
  kind: "edit",
  status: "completed",
  locations: [{ path: "src/runtime/cursor.rs", line: 41 }],
  content: [
    {
      type: "text",
      text:
        "diff src/runtime/cursor.rs\n" +
        "@@ -41,3 +41,3 @@\n" +
        "-let cursor = raw.strip_suffix('\\n').unwrap_or(raw);\n" +
        "+let cursor = raw.trim_end_matches(['\\r', '\\n']);\n" +
        " validate_cursor(cursor)?;",
    },
  ],
};
snap.timeline.push({ item: "tool_call", id: "edit", run });
snap.pending_permission = {
  id: "apply-cursor-fix",
  session: "flagship-session",
  tool_call: "edit",
  title: "Apply the cursor normalization fix?",
  detail:
    "diff src/runtime/cursor.rs\n" +
    "@@ -41,3 +41,3 @@\n" +
    "-let cursor = raw.strip_suffix('\\n').unwrap_or(raw);\n" +
    "+let cursor = raw.trim_end_matches(['\\r', '\\n']);\n" +
    " validate_cursor(cursor)?;",
  risk: "safe",
  reason: "One scoped source edit",
  options: [
    { id: "allow_once", label: "Allow once", kind: "allow_once" },
    { id: "reject_once", label: "Reject", kind: "reject_once" },
  ],
};
await push(snap);
await pause(900);

const allow = page.getByRole("button", { name: "Allow once" });
if (await allow.count()) {
  const box = await allow.boundingBox();
  if (box) await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 20 });
  await pause(350);
  await allow.click();
}
snap.pending_permission = undefined;
await push(snap);
await pause(700);

snap.tool_calls["local-test"] = {
  id: "local-test",
  title: "Run reconnect tests locally",
  kind: "execute",
  status: "in_progress",
  locations: [],
  content: [],
};
snap.timeline.push({ item: "tool_call", id: "local-test", run });
await push(snap);
await pause(1300);
snap.tool_calls["local-test"] = {
  ...snap.tool_calls["local-test"],
  title: "Run reconnect tests locally — 14 passed",
  status: "completed",
  content: [{ type: "text", text: "test result: ok. 14 passed; 0 failed; finished in 4.8s" }],
};
await push(snap);
await pause(400);
const localTest = page
  .locator('button[aria-expanded="false"]')
  .filter({ hasText: /14 passed/ })
  .last();
if (await localTest.count()) {
  const box = await localTest.boundingBox();
  if (box) await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 14 });
  await localTest.click();
}
await pause(1100);

snap.tool_calls["runner-test"] = {
  id: "runner-test",
  title: "Verify on remote runner",
  kind: "execute",
  status: "in_progress",
  locations: [],
  content: [],
};
snap.timeline.push({ item: "tool_call", id: "runner-test", run });
await push(snap);
await pause(1500);
snap.tool_calls["runner-test"] = {
  ...snap.tool_calls["runner-test"],
  title: "Verify on remote runner — Windows fixture passed",
  status: "completed",
  content: [
    {
      type: "text",
      text:
        "remote: windows-qa\n" +
        "reconnects_from_windows_receipt ... ok\n" +
        "reconnects_after_sleep ... ok\n" +
        "result: 14 passed; 0 failed; finished in 11.6s",
    },
  ],
};
snap.execution_checklist = {
  revision: 3,
  steps: [
    { title: "Reproduce from the reconnect log", status: "completed" },
    { title: "Trace cursor persistence across platforms", status: "completed" },
    { title: "Verify locally and on the runner", status: "completed" },
  ],
};
checklist.checklist = structuredClone(snap.execution_checklist);
snap.timeline.push({
  item: "message",
  run,
  role: "agent",
  phase: "final_answer",
  blocks: [
    {
      type: "text",
      text:
        "Fixed the reconnect loop at the first broken boundary: Windows `\\r\\n` receipts left `\\r` on the opaque resume cursor. One line changed. The original fixture now passes locally and on the remote Windows runner — **14 passed, 0 failed**.",
    },
  ],
});
snap.runs[run] = {
  id: run,
  status: "done",
  checkpoint: "before-reconnect-fix",
  outcome: {
    status: "done",
    stop_reason: "end_turn",
    execution: {
      execution_id: "flagship-execution",
      root_path: "/Users/demo/northstar-desktop",
      attempts: 1,
      recoveries: 0,
      child_executions: 3,
      completed_children: 3,
      failed_children: 0,
      weighted_tokens: 18_420,
      cost_usd: 0.18,
      changed_paths: ["src/runtime/cursor.rs"],
      completed_tools: ["read_file", "research", "edit_file", "shell"],
      failed_tools: [],
    },
  },
};
await push(snap);
await pause(3500);

const video = page.video();
await context.close();
await browser.close();
if (!video) throw new Error("Playwright did not produce a video");
await rename(await video.path(), TAKE);
console.log(`VIDEO: ${TAKE}`);
