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
import { ubuntuBuildInstallLaunchProbe } from "./utm-ubuntu-journey-probe.mjs";
import {
  seedAndLaunchUbuntuAuthenticatedWorkspace,
} from "./utm-ubuntu-webview.mjs";
import { captureUtmWindowObservation } from "./utm-window-observation.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const VM_NAME = "Clark QA - Ubuntu 24.04 Desktop";
const QMP_PORT = 47_112;

const ocrSource = String.raw`
import Foundation
import ImageIO
import Vision

guard CommandLine.arguments.count == 2 else {
    fatalError("expected one image path")
}
let url = URL(fileURLWithPath: CommandLine.arguments[1]) as CFURL
guard
    let source = CGImageSourceCreateWithURL(url, nil),
    let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
else {
    fatalError("cannot load image")
}
let request = VNRecognizeTextRequest()
request.recognitionLevel = .accurate
let handler = VNImageRequestHandler(cgImage: image)
try handler.perform([request])
let lines = (request.results ?? []).compactMap {
    $0.topCandidates(1).first?.string
}
print(lines.joined(separator: "\n"))
`;

function run(command, args, options = {}) {
  const completed = spawnSync(command, args, {
    encoding: "utf8",
    input: options.input,
    timeout: options.timeout_ms ?? 3_600_000,
    maxBuffer: 16 * 1024 * 1024,
  });
  return {
    ok: completed.status === 0,
    exit_code: completed.status,
    stdout: completed.stdout || "",
    stderr: completed.stderr || completed.error?.message || "",
  };
}

function redact(value) {
  return String(value)
    .replace(/\bck_(?:live|test)_[A-Za-z0-9._-]+\b/g, "ck_[REDACTED]")
    .replace(/\bsk-[A-Za-z0-9_-]{16,}\b/g, "sk-[REDACTED]")
    .replace(
      /\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b/g,
      "[JWT_REDACTED]",
    )
    .replace(/(authorization\s*[:=]\s*bearer\s+)\S+/gi, "$1[REDACTED]");
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
    throw new Error(`refusing to overwrite Ubuntu journey output ${outputDir}`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  chmodSync(outputDir, 0o700);
}

function visualContract(imagePath, { authenticated = false } = {}) {
  const recognized = run("swift", ["-", imagePath], {
    input: ocrSource,
    timeout_ms: 120_000,
  });
  const text = recognized.stdout.toLowerCase();
  const markers = authenticated
    ? {
        brand_visible: text.includes("clark code"),
        workspace_visible: text.includes("new session"),
        project_visible: text.includes("clarkcodeqa"),
        composer_controls_visible: (
          text.includes("describe what you want clark to do")
          && text.includes("approve for me")
          && text.includes("deepseek v4 flash latest")
        ),
        sign_in_absent: !text.includes("continue with google"),
        localhost_error_absent: !text.includes("could not connect to localhost"),
      }
    : {
        brand_visible: text.includes("clark code"),
        product_content_visible: [
          "continue with google",
          "coding agent on your machine",
          "new conversation",
          "clarkcodeqa",
        ].some((marker) => text.includes(marker)),
        localhost_error_absent: !text.includes("could not connect to localhost"),
      };
  return {
    status: (
      recognized.ok
      && Object.values(markers).every(Boolean)
    ) ? "passed" : "failed",
    transport: "macos_vision_ocr",
    markers,
    recognized_text_recorded: false,
    error: recognized.ok ? null : redact(recognized.stderr),
  };
}

function executeBuildInstallLaunch() {
  return executeGuestJson({
    platform: "ubuntu",
    vmName: VM_NAME,
    state: "started",
    probeSource: ubuntuBuildInstallLaunchProbe(),
    run,
    timeoutMs: 3_600_000,
    pollAttempts: 900,
    pollDelayMs: 5_000,
    executionAttempts: 1,
    detached: true,
  });
}

function sanitizedGuest(execution) {
  return execution.ok
    ? JSON.parse(redact(JSON.stringify(execution.data)))
    : {
        status: "failed",
        error: redact(execution.error),
        attempts: execution.attempts,
      };
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

export async function runUbuntuProductSmoke({ outputDir }) {
  prepareOutput(outputDir);
  const execution = executeBuildInstallLaunch();
  let observation = null;
  let visual = null;
  if (execution.ok && execution.data?.status === "passed") {
    const evidenceDir = path.join(outputDir, "evidence");
    observation = await captureUtmWindowObservation({
      platform: "ubuntu",
      vmName: VM_NAME,
      qmpPort: QMP_PORT,
      outputDir: evidenceDir,
    });
    if (observation.gui_visible) {
      visual = visualContract(path.join(evidenceDir, "ubuntu.png"));
    }
  }
  const guest = sanitizedGuest(execution);
  const passed = (
    guest.status === "passed"
    && observation?.gui_visible === true
    && visual?.status === "passed"
  );
  const receipt = {
    schema_version: 1,
    benchmark: "clark_code_ubuntu_native_product_smoke",
    status: passed ? "passed" : "failed",
    generated_at: new Date().toISOString(),
    source_revision: sourceRevision(),
    platform: "ubuntu",
    virtualization: "utm",
    vm_name: VM_NAME,
    required_user_vm_actions: 0,
    manual_vm_actions_allowed: false,
    human_input_observed: false,
    credential_recorded: false,
    paid_calls_made: false,
    guest,
    observation,
    visual_contract: visual,
    cleanup: {
      app_left_running_for_followup: guest.process_running === true,
    },
  };
  const receiptPath = path.join(outputDir, "receipt.json");
  writeFileSync(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o600 });
  console.log(JSON.stringify({
    status: receipt.status,
    architecture: guest.architecture || null,
    source_sha256: guest.source_sha256 || null,
    process_running: guest.process_running === true,
    window_visible: guest.window_visible === true,
    visual_contract: visual?.status || "not_run",
    required_user_vm_actions: 0,
  }));
  console.log(`RECEIPT=${receiptPath}`);
  if (!passed) process.exitCode = 1;
}

export async function runUbuntuAuthenticatedSmoke({ outputDir }) {
  prepareOutput(outputDir);
  const buildExecution = executeBuildInstallLaunch();
  const buildGuest = sanitizedGuest(buildExecution);
  let minted = null;
  let workspace = null;
  let observation = null;
  let visual = null;
  if (buildGuest.status === "passed") {
    minted = await mintClarkQaSession();
    workspace = seedAndLaunchUbuntuAuthenticatedWorkspace({
      retainedAuth: minted.retained_auth,
      run,
    });
    if (workspace.status === "passed") {
      const evidenceDir = path.join(outputDir, "evidence");
      observation = await captureUtmWindowObservation({
        platform: "ubuntu",
        vmName: VM_NAME,
        qmpPort: QMP_PORT,
        outputDir: evidenceDir,
      });
      if (observation.gui_visible) {
        visual = visualContract(path.join(evidenceDir, "ubuntu.png"), {
          authenticated: true,
        });
      }
    }
  }
  const passed = (
    buildGuest.status === "passed"
    && workspace?.status === "passed"
    && workspace?.workspace?.email_domain === "clarkslabs.com"
    && workspace?.sensitive_transfer_erased === true
    && observation?.gui_visible === true
    && visual?.status === "passed"
  );
  const receipt = {
    schema_version: 1,
    benchmark: "clark_code_ubuntu_authenticated_product_smoke",
    status: passed ? "passed" : "failed",
    generated_at: new Date().toISOString(),
    source_revision: sourceRevision(),
    platform: "ubuntu",
    virtualization: "utm",
    vm_name: VM_NAME,
    required_user_vm_actions: 0,
    manual_vm_actions_allowed: false,
    human_input_observed: false,
    credential_recorded: false,
    paid_calls_made: false,
    auth: minted
      ? {
          account_fingerprint: minted.account_fingerprint,
          email_domain: minted.account.email.split("@").at(-1).toLowerCase(),
          issuer: minted.issuer,
          expires_in_seconds_at_mint: minted.expires_in_seconds,
          transport: "better_auth_email_to_short_lived_jwt",
        }
      : null,
    build_install: buildGuest,
    workspace,
    observation,
    visual_contract: visual,
    cleanup: {
      transient_auth_transfer_erased:
        workspace?.sensitive_transfer_erased === true,
      app_left_running_for_followup: workspace?.process_running === true,
    },
  };
  const receiptPath = path.join(outputDir, "receipt.json");
  writeFileSync(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o600 });
  console.log(JSON.stringify({
    status: receipt.status,
    source_sha256: buildGuest.source_sha256 || null,
    auth_domain: receipt.auth?.email_domain || null,
    workspace_ready: workspace?.status === "passed",
    account_bound: workspace?.workspace?.account_bound === true,
    credential_fields_absent:
      workspace?.workspace?.credential_fields_absent === true,
    sensitive_transfer_erased:
      workspace?.sensitive_transfer_erased === true,
    visual_contract: visual?.status || "not_run",
    required_user_vm_actions: 0,
  }));
  console.log(`RECEIPT=${receiptPath}`);
  if (!passed) process.exitCode = 1;
}

async function runCli() {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    console.log(`Autonomous Ubuntu ARM Clark Code product journey

Usage:
  node harness/utm-ubuntu-journey.mjs build-smoke [--out NEW_DIRECTORY]
  node harness/utm-ubuntu-journey.mjs auth-smoke [--out NEW_DIRECTORY]

The journey builds the staged source with embedded Tauri assets, atomically
installs a native ARM debug product, unlocks the existing GNOME QA session,
launches as the home user, and validates visible Clark product content with
host OCR. auth-smoke then injects a short-lived Clark-owned QA session through
an erased transient channel, verifies same-account API-key provisioning, and
captures the authenticated workspace. Both require zero physical input.`);
    return;
  }
  if (!["build-smoke", "auth-smoke"].includes(args[0])) {
    throw new Error(`unknown command ${JSON.stringify(args[0])}`);
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
      || path.join("target", "utm-ubuntu-journey", `${stamp}-${process.pid}`),
  );
  if (args[0] === "build-smoke") {
    await runUbuntuProductSmoke({ outputDir });
  } else {
    await runUbuntuAuthenticatedSmoke({ outputDir });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await runCli();
}
