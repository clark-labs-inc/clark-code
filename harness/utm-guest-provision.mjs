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

import { executeGuestJson } from "./utm-guest-channel.mjs";
import {
  ubuntuProvisionProbe,
  windowsPowerShellParserProbe,
  windowsProvisionProbe,
} from "./utm-guest-provision-scripts.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const GUESTS = {
  windows: {
    vm_name: "Clark QA - Windows 11 ARM",
    probe: windowsProvisionProbe,
  },
  ubuntu: {
    vm_name: "Clark QA - Ubuntu 24.04 Desktop",
    probe: ubuntuProvisionProbe,
  },
};

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

export function provisionGuest(platform) {
  const guest = GUESTS[platform];
  if (!guest) throw new Error(`unsupported provisioning platform ${platform}`);
  const probeSource = guest.probe();
  if (platform === "windows") {
    const preflight = executeGuestJson({
      platform,
      vmName: guest.vm_name,
      state: "started",
      probeSource: windowsPowerShellParserProbe(probeSource),
      run,
      timeoutMs: 180_000,
      pollAttempts: 100,
      pollDelayMs: 100,
      executionAttempts: 2,
    });
    if (!preflight.ok) {
      return {
        platform,
        vm_name: guest.vm_name,
        status: "failed",
        phase: "powershell_parser_preflight",
        attempts: preflight.attempts,
        error: preflight.error,
      };
    }
    if (preflight.data.syntax_valid !== true) {
      return {
        platform,
        vm_name: guest.vm_name,
        status: "failed",
        phase: "powershell_parser_preflight",
        attempts: preflight.attempts,
        error: "Windows provisioning PowerShell failed syntax preflight",
        parser_errors: Array.isArray(preflight.data.errors)
          ? preflight.data.errors.slice(0, 20)
          : [],
      };
    }
  }
  const result = executeGuestJson({
    platform,
    vmName: guest.vm_name,
    state: "started",
    probeSource,
    run,
    timeoutMs: platform === "windows" ? 180_000 : 3_600_000,
    pollAttempts: platform === "windows" ? 720 : 400,
    pollDelayMs: platform === "windows" ? 5_000 : 500,
    executionAttempts: platform === "windows" ? 1 : 2,
    detached: platform === "windows",
  });
  if (!result.ok) {
    return {
      platform,
      vm_name: guest.vm_name,
      status: "failed",
      attempts: result.attempts,
      error: result.error,
    };
  }
  const data = result.data;
  const passed = (
    data.platform === platform
    && data.source_present === true
    && typeof data.node_version === "string"
    && typeof data.pnpm_version === "string"
    && typeof data.rustc_version === "string"
    && typeof data.cargo_version === "string"
    && (
      platform !== "ubuntu"
      || (
        Boolean(data.bubblewrap_path && data.webkit_pkg_version)
        && data.bubblewrap_sandbox_ready === true
        && data.apparmor_userns_restriction === "1"
      )
    )
    && (
      platform !== "windows"
      || (
        data.visual_studio_build_tools === true
        && data.msvc_toolset_count > 0
        && data.msvc_arm64_tools === true
        && typeof data.clang_path === "string"
        && data.clang_path.toLowerCase().endsWith("clang.exe")
        && data.rustc_host === "aarch64-pc-windows-msvc"
      )
    )
  );
  return {
    platform,
    vm_name: guest.vm_name,
    status: passed ? "passed" : "failed",
    attempts: result.attempts,
    data,
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

function prepareOutput(outputDir) {
  try {
    accessSync(outputDir);
    throw new Error(`refusing to overwrite guest provisioning output ${outputDir}`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  chmodSync(outputDir, 0o700);
}

async function runCli() {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    console.log(`Autonomous Clark Code UTM guest provisioning

Usage:
  node harness/utm-guest-provision.mjs ensure --platform windows|ubuntu|all
    [--out NEW_DIRECTORY]

Provisioning installs pinned, hash-verified Node and rustup distributions plus
official OS build dependencies. Windows additionally verifies the Microsoft
signature on Visual Studio Build Tools. All commands run through the UTM guest
agent and require zero user actions.`);
    return;
  }
  if (args[0] !== "ensure") throw new Error(`unknown command ${JSON.stringify(args[0])}`);
  for (let index = 1; index < args.length; index += 1) {
    const arg = args[index];
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
      || path.join("target", "utm-provision", `${stamp}-${process.pid}`),
  );
  prepareOutput(outputDir);
  const guests = [];
  for (const platform of platforms) {
    guests.push(provisionGuest(platform));
  }
  const receipt = {
    schema_version: 1,
    benchmark: "clark_code_utm_guest_provisioning",
    status: guests.every((guest) => guest.status === "passed") ? "passed" : "failed",
    generated_at: new Date().toISOString(),
    virtualization: "utm",
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
