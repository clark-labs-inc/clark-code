#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
export const NATIVE_CONTAINMENT_ASSERTIONS = [
  "project_write_allowed",
  "outside_write_blocked",
  "network_blocked",
];

function exactRevision(value) {
  if (!/^[0-9a-f]{40}$/.test(value)) {
    throw new Error("source revision must be a clean 40-character Git revision");
  }
  return value;
}

export function validateNativeContainmentReceipt(receipt, expectedRevision) {
  exactRevision(expectedRevision);
  const passed = new Set(
    (receipt?.assertions || [])
      .filter((item) => item.status === "passed")
      .map((item) => item.id),
  );
  if (
    receipt?.receipt_type !== "agent_windows_native_containment"
    || receipt?.status !== "passed"
    || receipt?.source_revision !== expectedRevision
    || !/^[0-9a-f]{64}$/.test(receipt?.evidence?.log_sha256 || "")
    || NATIVE_CONTAINMENT_ASSERTIONS.some((id) => !passed.has(id))
  ) {
    throw new Error("Windows native containment receipt is missing, stale, or failed");
  }
  return receipt;
}

export function runNativeContainment({
  runner,
  setup,
  sourceRevision,
  outputDir,
}) {
  exactRevision(sourceRevision);
  if (process.platform !== "win32") {
    throw new Error("native Windows containment verification must run on Windows");
  }
  if (existsSync(outputDir)) {
    throw new Error(`refusing to overwrite existing output directory ${outputDir}`);
  }
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  chmodSync(outputDir, 0o700);
  const completed = spawnSync(
    "cargo",
    [
      "test",
      "-p",
      "exec-sandbox-windows",
      "--test",
      "windows_native",
      "--",
      "--nocapture",
    ],
    {
      cwd: repoDir,
      env: {
        ...process.env,
        AGENT_WINDOWS_SANDBOX_E2E_REQUIRED: "1",
        AGENT_WINDOWS_SANDBOX_RUNNER: path.resolve(runner),
        AGENT_WINDOWS_SANDBOX_SETUP: path.resolve(setup),
      },
      encoding: "utf8",
      timeout: 10 * 60_000,
      maxBuffer: 32 * 1024 * 1024,
    },
  );
  const log = `${completed.stdout || ""}${completed.stderr || completed.error?.message || ""}`;
  const logPath = path.join(outputDir, "cargo-test.log");
  writeFileSync(logPath, log, { encoding: "utf8", mode: 0o600 });
  const corePassed = /agent_windows_core_containment=passed/.test(log);
  const passed = completed.status === 0
    && corePassed
    && /native_windows_sandbox_enforces_filesystem_process_and_network_boundaries \.\.\. ok/.test(log);
  const gitCompatibility = /agent_windows_git_compatibility=passed/.test(log)
    ? "passed"
    : /agent_windows_git_compatibility=failed_(?:optional|required)/.test(log)
      ? "failed"
      : "not_run";
  const receipt = {
    schema_version: 1,
    receipt_type: "agent_windows_native_containment",
    status: passed ? "passed" : "failed",
    generated_at: new Date().toISOString(),
    source_revision: sourceRevision,
    platform: "windows",
    required_user_actions: 0,
    assertions: NATIVE_CONTAINMENT_ASSERTIONS.map((id) => ({
      id,
      status: corePassed ? "passed" : "failed",
    })),
    evidence: {
      log: path.basename(logPath),
      log_sha256: createHash("sha256").update(log).digest("hex"),
      exit_code: completed.status,
      timed_out: completed.signal === "SIGTERM" && completed.error?.code === "ETIMEDOUT",
      git_compatibility: gitCompatibility,
    },
  };
  const receiptPath = path.join(outputDir, "receipt.json");
  writeFileSync(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, {
    encoding: "utf8",
    mode: 0o600,
  });
  chmodSync(receiptPath, 0o600);
  if (!passed) {
    throw new Error(`native Windows containment test failed; receipt: ${receiptPath}`);
  }
  return { receipt, receiptPath };
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

function main() {
  const args = process.argv.slice(2);
  const runner = valueArg(args, "--runner");
  const setup = valueArg(args, "--setup");
  const sourceRevision = valueArg(args, "--source-revision");
  const outputDir = valueArg(args, "--out");
  if (!runner || !setup || !sourceRevision || !outputDir) {
    throw new Error(
      "usage: windows-native-containment.mjs --runner FILE --setup FILE --source-revision SHA --out NEW_DIRECTORY",
    );
  }
  const result = runNativeContainment({
    runner,
    setup,
    sourceRevision,
    outputDir: path.resolve(outputDir),
  });
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`${error.stack || error.message}\n`);
    process.exitCode = 1;
  }
}
