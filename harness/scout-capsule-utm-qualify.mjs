#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import process from "node:process";

import { executeGuestJson } from "./utm-guest-channel.mjs";

const TARGETS = {
  ubuntu: {
    vm: "Clark QA - Ubuntu 24.04 Desktop",
    path(marker, extension) {
      return `/var/tmp/clark-scout-capsule-${marker}${extension}`;
    },
  },
  windows: {
    vm: "Clark QA - Windows 11 ARM",
    path(marker, extension) {
      return `C:\\Users\\Public\\clark-scout-capsule-${marker}${extension}`;
    },
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

function runBytes(command, args, options = {}) {
  const completed = spawnSync(command, args, {
    encoding: null,
    input: options.input,
    timeout: options.timeout_ms ?? 180_000,
    maxBuffer: 64 * 1024 * 1024,
  });
  if (completed.status !== 0) {
    throw new Error(
      String(
        completed.stderr?.toString("utf8")
          || completed.stdout?.toString("utf8")
          || completed.error?.message
          || `${command} failed`,
      ).slice(-4_000),
    );
  }
  return completed.stdout || Buffer.alloc(0);
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function py(value) {
  return JSON.stringify(String(value));
}

function ps(value) {
  return `'${String(value).replaceAll("'", "''")}'`;
}

function executionProbe(platform, binary, module, receipt) {
  if (platform === "ubuntu") {
    return `import hashlib, json, pathlib, subprocess
binary = pathlib.Path(${py(binary)})
module = pathlib.Path(${py(module)})
receipt = pathlib.Path(${py(receipt)})
binary.chmod(0o700)
completed = subprocess.run(
    [str(binary), str(module), str(receipt)],
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True,
    timeout=120,
)
document = json.loads(receipt.read_text(encoding="utf-8")) if receipt.exists() else {}
payload = {
    "exit_code": completed.returncode,
    "receipt_present": receipt.exists(),
    "receipt_sha256": hashlib.sha256(receipt.read_bytes()).hexdigest() if receipt.exists() else None,
    "guest_output_matches_native": document.get("guest_output_matches_native"),
    "module_sha256": document.get("isolation", {}).get("module_sha256"),
    "import_set": document.get("isolation", {}).get("import_set"),
    "fresh_instance": document.get("isolation", {}).get("fresh_instance"),
    "wasi_enabled": document.get("isolation", {}).get("wasi_enabled"),
    "signed_service_output_matches_native": document.get("signed_service", {}).get("output_matches_native"),
    "signed_service_module_sha256": document.get("signed_service", {}).get("isolation", {}).get("module_sha256"),
    "signed_registry_generation": document.get("signed_service", {}).get("generation"),
    "signed_deadline_is_hard_interrupt": document.get("signed_service", {}).get("deadline_is_hard_interrupt"),
}`;
  }
  return `$binary = ${ps(binary)}
$module = ${ps(module)}
$receipt = ${ps(receipt)}
$processError = $null
$child = $null
try {
  $child = Start-Process -FilePath $binary -ArgumentList @($module, $receipt) -Wait -PassThru -WindowStyle Hidden
} catch {
  $processError = [string]$_.Exception.Message
}
$document = if (Test-Path -LiteralPath $receipt) {
  Get-Content -LiteralPath $receipt -Raw | ConvertFrom-Json
} else { $null }
$payload = [ordered]@{
  process_error = $processError
  exit_code = if ($null -ne $child) { $child.ExitCode } else { $null }
  receipt_present = [bool](Test-Path -LiteralPath $receipt)
  receipt_sha256 = if (Test-Path -LiteralPath $receipt) {
    (Get-FileHash -LiteralPath $receipt -Algorithm SHA256).Hash.ToLowerInvariant()
  } else { $null }
  guest_output_matches_native = if ($null -ne $document) {
    [bool]$document.guest_output_matches_native
  } else { $null }
  module_sha256 = if ($null -ne $document) { [string]$document.isolation.module_sha256 } else { $null }
  import_count = if ($null -ne $document) { @($document.isolation.import_set).Count } else { $null }
  fresh_instance = if ($null -ne $document) { [bool]$document.isolation.fresh_instance } else { $null }
  wasi_enabled = if ($null -ne $document) { [bool]$document.isolation.wasi_enabled } else { $null }
  signed_service_output_matches_native = if ($null -ne $document) {
    [bool]$document.signed_service.output_matches_native
  } else { $null }
  signed_service_module_sha256 = if ($null -ne $document) {
    [string]$document.signed_service.isolation.module_sha256
  } else { $null }
  signed_registry_generation = if ($null -ne $document) {
    [int64]$document.signed_service.generation
  } else { $null }
  signed_deadline_is_hard_interrupt = if ($null -ne $document) {
    [bool]$document.signed_service.deadline_is_hard_interrupt
  } else { $null }
}`;
}

function securityProbe(binary, module) {
  return `$paths = @(${ps(binary)}, ${ps(module)})
$resources = @($paths | ForEach-Object { "file:_" + $_ })
$detections = @(
  Get-MpThreatDetection -ErrorAction SilentlyContinue |
    Where-Object {
      $entry = @($_.Resources)
      @($resources | Where-Object { $entry -contains $_ }).Count -gt 0
    } |
    Select-Object InitialDetectionTime, ThreatID, ActionSuccess, Resources
)
$threatIds = @($detections | ForEach-Object { $_.ThreatID } | Sort-Object -Unique)
$threats = @(
  Get-MpThreat -ErrorAction SilentlyContinue |
    Where-Object { $threatIds -contains $_.ThreatID } |
    Select-Object ThreatID, ThreatName, IsActive
)
$defender = Get-MpComputerStatus -ErrorAction SilentlyContinue
$signature = Get-AuthenticodeSignature -LiteralPath ${ps(binary)}
$payload = [ordered]@{
  defender_service = if ($null -ne $defender) { [bool]$defender.AMServiceEnabled } else { $null }
  realtime_protection = if ($null -ne $defender) { [bool]$defender.RealTimeProtectionEnabled } else { $null }
  signature_status = [string]$signature.Status
  detections = $detections
  threats = $threats
}`;
}

function cleanupProbe(platform, paths) {
  if (platform === "ubuntu") {
    return `import pathlib
paths = [pathlib.Path(value) for value in ${JSON.stringify(paths)}]
for item in paths:
    if item.exists():
        item.unlink()
payload = {"removed": all(not item.exists() for item in paths)}`;
  }
  return `$paths = @(${paths.map(ps).join(", ")})
Remove-Item -LiteralPath $paths -Force -ErrorAction SilentlyContinue
$payload = [ordered]@{
  removed = @($paths | Where-Object { Test-Path -LiteralPath $_ }).Count -eq 0
}`;
}

function valueArg(args, name) {
  const inline = args.find((argument) => argument.startsWith(`${name}=`));
  if (inline) return inline.slice(name.length + 1);
  const index = args.indexOf(name);
  if (index < 0) return undefined;
  const value = args[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value`);
  return value;
}

function qualify(platform, binaryPath, modulePath, outputDir) {
  const target = TARGETS[platform];
  if (!target) throw new Error("--platform must be ubuntu or windows");
  if (existsSync(outputDir)) throw new Error(`refusing to overwrite ${outputDir}`);
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });

  const binary = readFileSync(binaryPath);
  const module = readFileSync(modulePath);
  const binarySha = sha256(binary);
  const moduleSha = sha256(module);
  const marker = randomBytes(12).toString("hex");
  const guestBinary = target.path(marker, platform === "windows" ? ".exe" : "");
  const guestModule = target.path(marker, ".wasm");
  const guestReceipt = target.path(marker, ".json");
  const validation = {
    schema: "scout-capsule-utm-qualification-v2",
    status: "failed",
    platform,
    vm_name: target.vm,
    binary_sha256: binarySha,
    module_sha256: moduleSha,
    transfer: null,
    execution: null,
    security: null,
    cleanup: null,
    error: null,
  };

  try {
    runBytes("utmctl", ["file", "push", target.vm, guestBinary], { input: binary });
    runBytes("utmctl", ["file", "push", target.vm, guestModule], { input: module });
    const binaryBack = runBytes("utmctl", ["file", "pull", target.vm, guestBinary]);
    const moduleBack = runBytes("utmctl", ["file", "pull", target.vm, guestModule]);
    validation.transfer = {
      binary_read_back_matches: sha256(binaryBack) === binarySha,
      module_read_back_matches: sha256(moduleBack) === moduleSha,
    };
    if (!Object.values(validation.transfer).every(Boolean)) {
      throw new Error("UTM transfer read-back digest mismatch");
    }

    validation.execution = executeGuestJson({
      platform,
      vmName: target.vm,
      state: "started",
      probeSource: executionProbe(platform, guestBinary, guestModule, guestReceipt),
      run,
      timeoutMs: 180_000,
      executionAttempts: 1,
    });
    const execution = validation.execution.data;
    if (
      !validation.execution.ok
      || execution?.exit_code !== 0
      || execution?.receipt_present !== true
      || execution?.guest_output_matches_native !== true
      || execution?.module_sha256 !== moduleSha
      || execution?.fresh_instance !== true
      || execution?.wasi_enabled !== false
      || execution?.signed_service_output_matches_native !== true
      || execution?.signed_service_module_sha256 !== moduleSha
      || execution?.signed_registry_generation !== 7
      || execution?.signed_deadline_is_hard_interrupt !== false
      || (
        platform === "windows"
          ? execution?.import_count !== 0
          : execution?.import_set?.length !== 0
      )
    ) {
      throw new Error(validation.execution.error || "capsule execution gate failed");
    }
    const receipt = runBytes("utmctl", ["file", "pull", target.vm, guestReceipt]);
    if (sha256(receipt) !== execution.receipt_sha256) {
      throw new Error("capsule receipt changed while crossing the UTM boundary");
    }
    writeFileSync(path.join(outputDir, "receipt.json"), receipt, { mode: 0o600 });

    if (platform === "windows") {
      validation.security = executeGuestJson({
        platform,
        vmName: target.vm,
        state: "started",
        probeSource: securityProbe(guestBinary, guestModule),
        run,
        executionAttempts: 1,
      });
      const security = validation.security.data;
      if (
        !validation.security.ok
        || security?.defender_service !== true
        || security?.realtime_protection !== true
        || security?.detections?.length !== 0
        || security?.threats?.length !== 0
      ) {
        throw new Error(validation.security.error || "Windows Defender gate failed");
      }
    }
    validation.status = "passed";
  } catch (error) {
    validation.error = String(error.message || error).slice(-4_000);
  } finally {
    validation.cleanup = executeGuestJson({
      platform,
      vmName: target.vm,
      state: "started",
      probeSource: cleanupProbe(platform, [guestBinary, guestModule, guestReceipt]),
      run,
      executionAttempts: 1,
    });
    if (!validation.cleanup.ok || validation.cleanup.data?.removed !== true) {
      validation.status = "failed";
      validation.error ||= validation.cleanup.error || "guest scratch cleanup was not proven";
    }
    writeFileSync(
      path.join(outputDir, "validation.json"),
      `${JSON.stringify(validation, null, 2)}\n`,
      { mode: 0o600 },
    );
  }
  return validation;
}

const args = process.argv.slice(2);
if (args.includes("--help") || args.includes("-h")) {
  console.log("Usage: scout-capsule-utm-qualify.mjs --platform ubuntu|windows --binary PATH --module PATH --out NEW_DIRECTORY");
  process.exit(0);
}
const platform = valueArg(args, "--platform");
const binaryPath = path.resolve(valueArg(args, "--binary") || "");
const modulePath = path.resolve(valueArg(args, "--module") || "");
const outputDir = path.resolve(valueArg(args, "--out") || "");
if (!platform || !binaryPath || !modulePath || !outputDir) {
  throw new Error("--platform, --binary, --module, and --out are required");
}
const result = qualify(platform, binaryPath, modulePath, outputDir);
console.log(`validation=${path.join(outputDir, "validation.json")}`);
console.log(`status=${result.status}`);
if (result.status !== "passed") process.exitCode = 1;
