#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import {
  accessSync,
  chmodSync,
  mkdirSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

import { executeGuestJson } from "./utm-guest-channel.mjs";
import {
  ubuntuOfflineBenchmarkProbe,
  ubuntuPythonParserProbe,
  windowsOfflineBenchmarkProbe,
} from "./utm-guest-benchmark-scripts.mjs";
import { windowsPowerShellParserProbe } from "./utm-guest-provision-scripts.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const GUESTS = {
  windows: {
    vm_name: "Clark QA - Windows 11 ARM",
    probe: windowsOfflineBenchmarkProbe,
    report_prefix: "C:\\ClarkQA\\runs\\",
  },
  ubuntu: {
    vm_name: "Clark QA - Ubuntu 24.04 Desktop",
    probe: ubuntuOfflineBenchmarkProbe,
    report_prefix: "/opt/clark-qa/runs/",
  },
};

function run(command, args, options = {}) {
  const completed = spawnSync(command, args, {
    encoding: "utf8",
    input: options.input,
    timeout: options.timeout_ms ?? 180_000,
    maxBuffer: 64 * 1024 * 1024,
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
    .replace(/(authorization\s*[:=]\s*bearer\s+)\S+/gi, "$1[REDACTED]");
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function prepareOutput(outputDir) {
  try {
    accessSync(outputDir);
    throw new Error(`refusing to overwrite guest benchmark output ${outputDir}`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  chmodSync(outputDir, 0o700);
}

function pullReport({ platform, guest, runId, guestPath, expectedHash, outputDir }) {
  const expectedPrefix = `${guest.report_prefix}${runId}`;
  const normalizedPath = platform === "windows"
    ? String(guestPath).toLowerCase()
    : String(guestPath);
  const normalizedPrefix = platform === "windows"
    ? expectedPrefix.toLowerCase()
    : expectedPrefix;
  if (!normalizedPath.startsWith(normalizedPrefix) || !normalizedPath.endsWith("report.json")) {
    throw new Error(`${platform} returned an unsafe report path`);
  }
  const pulled = run(
    "utmctl",
    ["file", "pull", guest.vm_name, guestPath],
    { timeout_ms: 180_000 },
  );
  if (!pulled.ok) {
    throw new Error(`${platform} report pull failed: ${redact(pulled.stderr || pulled.stdout)}`);
  }
  const actualHash = sha256(pulled.stdout);
  if (actualHash !== expectedHash) {
    throw new Error(`${platform} report SHA-256 changed during export`);
  }
  const report = JSON.parse(pulled.stdout);
  if (
    report?.schema_version !== 2
    || report?.benchmark !== "clark_code_consolidated"
    || report?.platform !== platform
    || report?.execution?.mode !== "offline"
    || report?.execution?.credential_recorded !== false
  ) {
    throw new Error(`${platform} exported report violates the offline matrix contract`);
  }
  const serialized = `${JSON.stringify(report, null, 2)}\n`;
  if (/\bck_(?:live|test)_[A-Za-z0-9._-]+\b/.test(serialized)) {
    throw new Error(`${platform} exported report contains credential-shaped data`);
  }
  const relativePath = path.join(platform, "matrix-report.json");
  const hostPath = path.join(outputDir, relativePath);
  mkdirSync(path.dirname(hostPath), { recursive: true, mode: 0o700 });
  writeFileSync(hostPath, serialized, { mode: 0o600 });
  return { report, relative_path: relativePath, sha256: sha256(serialized) };
}

export function runGuestBenchmark({ platform, runId, outputDir }) {
  const guest = GUESTS[platform];
  if (!guest) throw new Error(`unsupported guest benchmark platform ${platform}`);
  const probeSource = guest.probe({ runId });
  const parserSource = platform === "windows"
    ? windowsPowerShellParserProbe(probeSource)
    : ubuntuPythonParserProbe(probeSource);
  const preflight = executeGuestJson({
    platform,
    vmName: guest.vm_name,
    state: "started",
    probeSource: parserSource,
    run,
    timeoutMs: 180_000,
    pollAttempts: 100,
    pollDelayMs: 100,
    executionAttempts: 2,
  });
  if (!preflight.ok || preflight.data.syntax_valid !== true) {
    return {
      platform,
      vm_name: guest.vm_name,
      status: "failed",
      phase: "guest_script_syntax_preflight",
      error: redact(preflight.error || "guest benchmark script failed syntax preflight"),
      parser_errors: preflight.data?.errors || [],
      attempts: preflight.attempts,
    };
  }
  const execution = executeGuestJson({
    platform,
    vmName: guest.vm_name,
    state: "started",
    probeSource,
    run,
    timeoutMs: 180_000,
    pollAttempts: 900,
    pollDelayMs: 5_000,
    executionAttempts: 1,
    detached: true,
  });
  if (!execution.ok) {
    return {
      platform,
      vm_name: guest.vm_name,
      status: "failed",
      error: redact(execution.error),
      attempts: execution.attempts,
    };
  }
  const data = execution.data;
  let exported = null;
  if (data.report_present && /^[a-f0-9]{64}$/.test(data.report_sha256 || "")) {
    exported = pullReport({
      platform,
      guest,
      runId,
      guestPath: data.report_path,
      expectedHash: data.report_sha256,
      outputDir,
    });
  }
  const passed = (
    data.status === "passed"
    && data.platform === platform
    && data.required_user_vm_actions === 0
    && data.manual_vm_actions_allowed === false
    && data.human_input_observed === false
    && data.credential_recorded === false
    && /^[a-f0-9]{64}$/.test(data.source_sha256 || "")
    && exported?.report.status === "passed"
  );
  return {
    platform,
    vm_name: guest.vm_name,
    status: passed ? "passed" : "failed",
    attempts: execution.attempts,
    execution_user: data.execution_user,
    source_sha256: data.source_sha256,
    report: exported
      ? {
          file: exported.relative_path,
          sha256: exported.sha256,
          guest_sha256: data.report_sha256,
          summary: exported.report.summary,
        }
      : null,
    steps: data.steps || [],
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

async function runCli() {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    console.log(`Clark Code deterministic UTM guest benchmark

Usage:
  node harness/utm-guest-benchmark.mjs run --offline
    [--platform windows|ubuntu|all] [--out NEW_DIRECTORY]

The command installs both frozen pnpm lockfiles, runs the full platform-specific
offline feature matrix in each selected guest, and exports a signed SHA-256
report. Long jobs are detached from UTM's synchronous event timeout. No guest
input or credential transfer is allowed.`);
    return;
  }
  if (args[0] !== "run") throw new Error(`unknown command ${JSON.stringify(args[0])}`);
  if (!args.includes("--offline")) {
    throw new Error("guest benchmark currently requires the explicit --offline deterministic gate");
  }
  const knownFlags = new Set(["--offline"]);
  for (let index = 1; index < args.length; index += 1) {
    const arg = args[index];
    if (knownFlags.has(arg)) continue;
    if (["--platform", "--out"].includes(arg)) {
      index += 1;
      continue;
    }
    if (["--platform=", "--out="].some((prefix) => arg.startsWith(prefix))) continue;
    throw new Error(`unknown argument ${JSON.stringify(arg)}`);
  }
  const selected = valueArg(args, "--platform") || "all";
  const platforms = selected === "all" ? ["windows", "ubuntu"] : [selected];
  if (platforms.some((platform) => !GUESTS[platform])) {
    throw new Error("--platform must be windows, ubuntu, or all");
  }
  const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
  const outputDir = path.resolve(
    repoDir,
    valueArg(args, "--out")
      || path.join("target", "utm-guest-benchmark", `${stamp}-${process.pid}`),
  );
  prepareOutput(outputDir);
  const runId = `offline-${randomBytes(8).toString("hex")}`;
  const guests = [];
  for (const platform of platforms) {
    guests.push(runGuestBenchmark({ platform, runId, outputDir }));
  }
  const sourceHashes = new Set(
    guests.map((guest) => guest.source_sha256).filter(Boolean),
  );
  const receipt = {
    schema_version: 1,
    benchmark: "clark_code_utm_guest_offline_matrix",
    status: (
      guests.every((guest) => guest.status === "passed")
      && sourceHashes.size === 1
    ) ? "passed" : "failed",
    generated_at: new Date().toISOString(),
    virtualization: "utm",
    mode: "offline",
    run_id: runId,
    required_user_vm_actions: 0,
    manual_vm_actions_allowed: false,
    human_input_observed: false,
    credential_recorded: false,
    guests,
  };
  const receiptPath = path.join(outputDir, "receipt.json");
  writeFileSync(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o600 });
  console.log(JSON.stringify({
    status: receipt.status,
    guests: Object.fromEntries(guests.map((guest) => [guest.platform, guest.status])),
    required_user_vm_actions: 0,
  }));
  console.log(`RECEIPT=${receiptPath}`);
  if (receipt.status !== "passed") process.exitCode = 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await runCli();
}
