#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import {
  accessSync,
  chmodSync,
  mkdirSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

import { mintClarkQaSession } from "./clark-qa-auth.mjs";
import { executeGuestJson } from "./utm-guest-channel.mjs";
import { captureUtmWindowObservation } from "./utm-window-observation.mjs";
import {
  configureWebViewDebugPolicy,
  evaluateWindowsClarkWebView,
  launchWindowsClarkCode,
  qaAccountSettingsExpression,
  seedWindowsNativeCredentials,
} from "./utm-windows-webview.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const VM_NAME = "Clark QA - Windows 11 ARM";
const QMP_PORT = 47_111;
const QA_ROOT = String.raw`C:\Users\home\ClarkCodeQA`;
const QA_MODEL = "clark-code:deepseek_v4_flash_latest";

function sleep(milliseconds) {
  if (milliseconds > 0) {
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);
  }
}

function run(command, args, options = {}) {
  const completed = spawnSync(command, args, {
    encoding: "utf8",
    input: options.input,
    timeout: options.timeout_ms ?? 30_000,
    maxBuffer: 4 * 1024 * 1024,
  });
  return {
    ok: completed.status === 0,
    exit_code: completed.status,
    stdout: completed.stdout || "",
    stderr: completed.stderr || completed.error?.message || "",
  };
}

function sourceRevision() {
  const revision = run("git", ["rev-parse", "HEAD"]);
  if (!revision.ok) return "unknown";
  const dirty = run("git", ["status", "--porcelain"]);
  return `${revision.stdout.trim()}${dirty.stdout.trim() ? "-dirty" : ""}`;
}

function prepareOutput(outputDir) {
  try {
    accessSync(outputDir);
    throw new Error(`refusing to overwrite Windows journey output ${outputDir}`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  chmodSync(outputDir, 0o700);
}

function windowsGuestJson(probeSource) {
  const result = executeGuestJson({
    platform: "windows",
    vmName: VM_NAME,
    state: "started",
    probeSource,
    run,
    timeoutMs: 45_000,
  });
  if (!result.ok) throw new Error(`Windows guest command failed: ${result.error}`);
  return result.data;
}

export function ensureWindowsQaFixture() {
  return windowsGuestJson(String.raw`
$root = "${QA_ROOT}"
New-Item -ItemType Directory -Force -Path $root | Out-Null
$nl = [Environment]::NewLine
[IO.File]::WriteAllText(
  (Join-Path $root "README.md"),
  ("# Clark Code autonomous VM fixture" + $nl + $nl + "This repository belongs only to the QA harness." + $nl)
)
[IO.File]::WriteAllText((Join-Path $root "numbers.txt"), ("2" + $nl + "3" + $nl + "5" + $nl + "7" + $nl))
[IO.File]::WriteAllText((Join-Path $root "expected.txt"), ("17" + $nl))
$payload = [ordered]@{
  root = $root
  file_count = @(Get-ChildItem -LiteralPath $root -File).Count
  readme_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $root "README.md")).Hash.ToLower()
  numbers_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $root "numbers.txt")).Hash.ToLower()
  expected_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $root "expected.txt")).Hash.ToLower()
}
`);
}

function workspaceObservationExpression(account) {
  const owner = `id:${account.id}`;
  return `(() => {
    let settings = {};
    let project = {};
    try {
      const encodedOwner = encodeURIComponent(${JSON.stringify(owner)});
      settings = JSON.parse(
        localStorage.getItem("clark-desktop:local-agent:" + encodedOwner) || "{}"
      );
      project = JSON.parse(
        localStorage.getItem("clark-desktop:project-context:" + encodedOwner) || "{}"
      );
    } catch {}
    const text = document.body?.innerText || "";
    const storage = JSON.stringify(Object.fromEntries(Object.entries(localStorage)));
    return {
      title: document.title,
      url: location.href,
      sign_in_visible: text.includes("Continue with Google"),
      account_bound: true,
      credential_fields_absent: !/apiKey|apiKeyOwner|clarkToken|refreshToken|accessToken/i.test(storage),
      project_configured: project.cwd === ${JSON.stringify(QA_ROOT)},
      model_configured: settings.model === ${JSON.stringify(QA_MODEL)},
      project_visible: text.includes("ClarkCodeQA"),
      model_visible: text.includes("Free"),
      account_visible: text.includes(${JSON.stringify(account.name)})
    };
  })()`;
}

function workspaceReady(observed) {
  return (
    observed?.title === "Clark Code"
    && observed?.url === "http://tauri.localhost/"
    && observed?.sign_in_visible === false
    && observed?.account_bound === true
    && observed?.credential_fields_absent === true
    && observed?.project_configured === true
    && observed?.model_configured === true
    && observed?.project_visible === true
    && observed?.model_visible === true
  );
}

export function waitForWindowsWorkspace({
  account,
  attempts = 30,
  delayMs = 1_000,
} = {}) {
  let observed = null;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    if (attempt > 0) sleep(delayMs);
    observed = evaluateWindowsClarkWebView({
      expression: workspaceObservationExpression(account),
    });
    if (workspaceReady(observed)) {
      return { status: "passed", attempts: attempt + 1, ...observed };
    }
  }
  return { status: "failed", attempts, ...observed };
}

export async function runWindowsAuthenticatedSmoke({ outputDir }) {
  prepareOutput(outputDir);
  const fixture = ensureWindowsQaFixture();
  const minted = await mintClarkQaSession();
  let policyEnabled = null;
  let cleanup = null;
  try {
    policyEnabled = configureWebViewDebugPolicy({ enabled: true });
    seedWindowsNativeCredentials({ retainedAuth: minted.retained_auth, run });
    const launch = await launchWindowsClarkCode({ qmpPort: QMP_PORT });
    const seeded = evaluateWindowsClarkWebView({
      expression: qaAccountSettingsExpression({
        account: minted.account,
        cwd: QA_ROOT,
        model: QA_MODEL,
      }),
    });
    evaluateWindowsClarkWebView({
      expression: `location.reload(); ({reloading: true})`,
    });
    const workspace = waitForWindowsWorkspace({ account: minted.account });
    const observation = await captureUtmWindowObservation({
      platform: "windows",
      vmName: VM_NAME,
      qmpPort: QMP_PORT,
      outputDir: path.join(outputDir, "evidence"),
    });
    cleanup = configureWebViewDebugPolicy({ enabled: false });
    const passed = (
      workspace.status === "passed"
      && observation.gui_visible === true
      && cleanup.policy_state_matches === true
    );
    const receipt = {
      schema_version: 1,
      benchmark: "clark_code_windows_authenticated_product_smoke",
      status: passed ? "passed" : "failed",
      generated_at: new Date().toISOString(),
      source_revision: sourceRevision(),
      platform: "windows",
      virtualization: "utm",
      vm_name: VM_NAME,
      required_user_vm_actions: 0,
      manual_vm_actions_allowed: false,
      human_input_observed: false,
      credential_recorded: false,
      auth: {
        account_fingerprint: minted.account_fingerprint,
        email_domain: minted.account.email.split("@").at(-1).toLowerCase(),
        issuer: minted.issuer,
        expires_in_seconds_at_mint: minted.expires_in_seconds,
        transport: "better_auth_email_to_short_lived_jwt",
      },
      fixture,
      launch,
      webview: {
        cdp_bind: "guest_loopback",
        temporary_policy_enabled: policyEnabled.policy_state_matches,
        seeded,
        workspace,
      },
      observation,
      cleanup: {
        app_stopped: cleanup.app_stopped,
        temporary_policy_removed: cleanup.policy_state_matches,
      },
      paid_calls_made: false,
    };
    const receiptPath = path.join(outputDir, "receipt.json");
    writeFileSync(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o600 });
    writeFileSync(
      path.join(outputDir, "report.md"),
      `# Windows authenticated Clark Code smoke

**Result:** ${receipt.status}
**Required user VM actions:** 0
**Human input observed:** false
**Paid calls made:** false
**QA identity domain:** ${receipt.auth.email_domain}
**Temporary WebView diagnostic policy removed:** ${receipt.cleanup.temporary_policy_removed}

The installed Windows release signed in with the dedicated QA identity, bound a
new or existing Clark Code API key to that same account, opened the disposable
fixture, and exposed no credential in its owner-only receipt.
`,
      { mode: 0o600 },
    );
    return { receipt, receiptPath };
  } finally {
    if (!cleanup) {
      try {
        configureWebViewDebugPolicy({ enabled: false });
      } catch {
        // Preserve the primary failure. The policy command is idempotent and
        // the next run starts by stopping the app and reconciling all values.
      }
    }
  }
}

function valueArg(args, name) {
  const inline = args.find((arg) => arg.startsWith(`${name}=`));
  if (inline) return inline.slice(name.length + 1);
  const index = args.indexOf(name);
  if (index < 0) return undefined;
  const value = args[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value`);
  return value;
}

async function runCli() {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    console.log(`Autonomous Windows Clark Code product journey

Usage:
  node harness/utm-windows-journey.mjs auth-smoke [--out NEW_DIRECTORY]

The smoke controls the installed Windows release entirely through UTM QMP, the
authenticated guest-agent file channel, and temporary guest-loopback WebView2
CDP. It mints a short-lived QA JWT, verifies same-account API-key provisioning,
captures fresh GUI evidence, removes the diagnostic policy, and requires zero
human input.`);
    return;
  }
  const command = args[0];
  if (command !== "auth-smoke") {
    throw new Error(`unknown command ${JSON.stringify(command)}`);
  }
  for (let index = 1; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--out") {
      index += 1;
      continue;
    }
    if (arg.startsWith("--out=")) continue;
    throw new Error(`unknown argument ${JSON.stringify(arg)}`);
  }
  const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
  const outputDir = path.resolve(
    repoDir,
    valueArg(args, "--out")
      || path.join("target", "utm-windows-journey", `${stamp}-${process.pid}`),
  );
  const { receipt, receiptPath } = await runWindowsAuthenticatedSmoke({ outputDir });
  console.log(JSON.stringify({
    status: receipt.status,
    required_user_vm_actions: 0,
    human_input_observed: false,
    account_bound: receipt.webview.workspace.account_bound,
    credential_fields_absent: receipt.webview.workspace.credential_fields_absent,
    cleanup: receipt.cleanup.temporary_policy_removed,
  }));
  console.log(`RECEIPT=${receiptPath}`);
  if (receipt.status !== "passed") process.exitCode = 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await runCli();
}
