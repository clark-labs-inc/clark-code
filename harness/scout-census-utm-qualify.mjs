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
  macos: {
    vm: "Clark QA - macOS 26",
    path(marker, extension) {
      return `/var/tmp/clark-scout-census-${marker}${extension}`;
    },
  },
  ubuntu: {
    vm: "Clark QA - Ubuntu 24.04 Desktop",
    path(marker, extension) {
      return `/var/tmp/clark-scout-census-${marker}${extension}`;
    },
  },
  windows: {
    vm: "Clark QA - Windows 11 ARM",
    path(marker, extension) {
      return `C:\\Users\\Public\\clark-scout-census-${marker}${extension}`;
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

function executionProbe(platform, binary, root, receipt, stderrPath) {
  const argumentsList = [
    "--root",
    root,
    "--max-depth",
    "64",
    "--max-directories",
    "100000",
    "--max-files",
    "65536",
    "--max-bytes",
    "268435456",
    "--max-file-bytes",
    "16777216",
    "--max-keys-per-file",
    "65536",
    "--pretty",
  ];
  if (platform === "ubuntu" || platform === "macos") {
    return `import hashlib, json, pathlib, subprocess
binary = pathlib.Path(${py(binary)})
root = pathlib.Path(${py(root)})
receipt = pathlib.Path(${py(receipt)})
stderr_path = pathlib.Path(${py(stderrPath)})
binary.chmod(0o700)
completed = subprocess.run(
    [str(binary), *${JSON.stringify(argumentsList)}],
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    timeout=180,
)
stderr_path.write_bytes(completed.stderr)
if completed.returncode == 0:
    receipt.write_bytes(completed.stdout)
document = json.loads(receipt.read_text(encoding="utf-8")) if receipt.exists() else {}
payload = {
    "exit_code": completed.returncode,
    "root_present": root.is_dir(),
    "receipt_present": receipt.exists(),
    "receipt_sha256": hashlib.sha256(receipt.read_bytes()).hexdigest() if receipt.exists() else None,
    "platform": document.get("platform"),
    "architecture": document.get("architecture"),
    "coverage": document.get("coverage"),
    "truncation": document.get("truncation"),
    "redaction": document.get("redaction"),
}`;
  }
  const argumentsPowerShell = argumentsList.map(ps).join(", ");
  return `$binary = ${ps(binary)}
$root = ${ps(root)}
$receipt = ${ps(receipt)}
$stderrPath = ${ps(stderrPath)}
Unblock-File -LiteralPath $binary -ErrorAction SilentlyContinue
$standardOutput = (& $binary @(${argumentsPowerShell}) 2> $stderrPath | Out-String)
$exitCode = $LASTEXITCODE
[IO.File]::WriteAllText(
  $receipt,
  $standardOutput,
  [Text.UTF8Encoding]::new($false)
)
$document = if ($exitCode -eq 0 -and (Test-Path -LiteralPath $receipt)) {
  Get-Content -LiteralPath $receipt -Raw | ConvertFrom-Json
} else { $null }
$payload = [ordered]@{
  exit_code = $exitCode
  root_present = [bool](Test-Path -LiteralPath $root -PathType Container)
  receipt_present = [bool](Test-Path -LiteralPath $receipt)
  receipt_sha256 = if (Test-Path -LiteralPath $receipt) {
    (Get-FileHash -LiteralPath $receipt -Algorithm SHA256).Hash.ToLowerInvariant()
  } else { $null }
  platform = if ($null -ne $document) { [string]$document.platform } else { $null }
  architecture = if ($null -ne $document) { [string]$document.architecture } else { $null }
  coverage = if ($null -ne $document) { $document.coverage } else { $null }
  truncation = if ($null -ne $document) { $document.truncation } else { $null }
  redaction = if ($null -ne $document) { $document.redaction } else { $null }
}`;
}

function windowsSecurityProbe(binary) {
  return `$binary = ${ps(binary)}
$resource = "file:_" + $binary
$detections = @(
  Get-MpThreatDetection -ErrorAction SilentlyContinue |
    Where-Object { @($_.Resources) -contains $resource } |
    Select-Object InitialDetectionTime, ThreatID, ActionSuccess, Resources
)
$threatIds = @($detections | ForEach-Object { $_.ThreatID } | Sort-Object -Unique)
$threats = @(
  Get-MpThreat -ErrorAction SilentlyContinue |
    Where-Object { $threatIds -contains $_.ThreatID } |
    Select-Object ThreatID, ThreatName, IsActive
)
$defender = Get-MpComputerStatus -ErrorAction SilentlyContinue
$signature = Get-AuthenticodeSignature -LiteralPath $binary
$payload = [ordered]@{
  defender_service = if ($null -ne $defender) { [bool]$defender.AMServiceEnabled } else { $null }
  realtime_protection = if ($null -ne $defender) { [bool]$defender.RealTimeProtectionEnabled } else { $null }
  signature_status = [string]$signature.Status
  detections = $detections
  threats = $threats
}`;
}

function cleanupProbe(platform, paths) {
  if (platform === "ubuntu" || platform === "macos") {
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

function hasTruncation(receipt) {
  return Object.values(receipt.truncation || {}).some((value) => value === true);
}

function qualify(platform, binaryPath, guestRoot, outputDir) {
  const target = TARGETS[platform];
  if (!target) throw new Error("--platform must be macos, ubuntu, or windows");
  if (existsSync(outputDir)) throw new Error(`refusing to overwrite ${outputDir}`);
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });

  const binary = readFileSync(binaryPath);
  const binarySha = sha256(binary);
  const marker = randomBytes(12).toString("hex");
  const guestBinary = target.path(marker, platform === "windows" ? ".exe" : "");
  const guestReceipt = target.path(marker, ".json");
  const guestStderr = target.path(marker, ".stderr");
  const validation = {
    schema_version: "scout-census-utm-qualification-v1",
    status: "failed",
    platform,
    vm_name: target.vm,
    guest_root: guestRoot,
    binary_sha256: binarySha,
    transfer: null,
    execution: null,
    security: null,
    cleanup: null,
    error: null,
  };

  try {
    runBytes("utmctl", ["file", "push", target.vm, guestBinary], { input: binary });
    const readBack = runBytes("utmctl", ["file", "pull", target.vm, guestBinary]);
    validation.transfer = {
      read_back_matches: sha256(readBack) === binarySha,
      read_back_sha256: sha256(readBack),
      read_back_length: readBack.length,
    };
    if (!validation.transfer.read_back_matches) {
      throw new Error("UTM transfer read-back digest mismatch");
    }

    validation.execution = executeGuestJson({
      platform,
      vmName: target.vm,
      state: "started",
      probeSource: executionProbe(
        platform,
        guestBinary,
        guestRoot,
        guestReceipt,
        guestStderr,
      ),
      run,
      timeoutMs: 180_000,
      executionAttempts: 1,
    });
    const execution = validation.execution.data;
    if (
      !validation.execution.ok
      || execution?.exit_code !== 0
      || execution?.root_present !== true
      || execution?.receipt_present !== true
      || execution?.redaction?.values_emitted !== false
      || execution?.redaction?.discovered_executables_executed !== false
      || hasTruncation(execution)
    ) {
      throw new Error(validation.execution.error || "guest census execution gate failed");
    }
    const receiptBytes = runBytes(
      "utmctl",
      ["file", "pull", target.vm, guestReceipt],
    );
    if (sha256(receiptBytes) !== execution.receipt_sha256) {
      throw new Error("census receipt changed while crossing the UTM boundary");
    }
    const receipt = JSON.parse(receiptBytes.toString("utf8").replace(/^\uFEFF/, ""));
    if (
      receipt.redaction?.values_emitted !== false
      || receipt.redaction?.discovered_executables_executed !== false
      || hasTruncation(receipt)
    ) {
      throw new Error("pulled census receipt failed the redaction or terminal-bounds gate");
    }
    writeFileSync(path.join(outputDir, "receipt.json"), receiptBytes, { mode: 0o600 });

    if (platform === "windows") {
      validation.security = executeGuestJson({
        platform,
        vmName: target.vm,
        state: "started",
        probeSource: windowsSecurityProbe(guestBinary),
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
      probeSource: cleanupProbe(
        platform,
        [guestBinary, guestReceipt, guestStderr],
      ),
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
  console.log(
    "Usage: scout-census-utm-qualify.mjs "
      + "--platform macos|ubuntu|windows --binary PATH "
      + "--root GUEST_PATH --out NEW_DIRECTORY",
  );
  process.exit(0);
}
const platform = valueArg(args, "--platform");
const binaryPath = path.resolve(valueArg(args, "--binary") || "");
const guestRoot = valueArg(args, "--root");
const outputDir = path.resolve(valueArg(args, "--out") || "");
if (!platform || !binaryPath || !guestRoot || !outputDir) {
  throw new Error("--platform, --binary, --root, and --out are required");
}
const result = qualify(platform, binaryPath, guestRoot, outputDir);
console.log(`validation=${path.join(outputDir, "validation.json")}`);
console.log(`status=${result.status}`);
if (result.status !== "passed") process.exitCode = 1;
