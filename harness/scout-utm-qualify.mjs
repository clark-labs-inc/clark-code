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

const GUESTS = {
  ubuntu: {
    vm_name: "Clark QA - Ubuntu 24.04 Desktop",
    binary(marker) {
      return `/var/tmp/clark-scout-${marker}`;
    },
    output(marker) {
      return `/var/tmp/clark-scout-${marker}-out`;
    },
    receipt(output) {
      return `${output}/receipt.json`;
    },
    report(output) {
      return `${output}/report.md`;
    },
  },
  windows: {
    vm_name: "Clark QA - Windows 11 ARM",
    binary(marker) {
      return `C:\\Users\\Public\\clark-scout-${marker}.exe`;
    },
    output(marker) {
      return `C:\\Users\\Public\\clark-scout-${marker}-out`;
    },
    receipt(output) {
      return `${output}\\receipt.json`;
    },
    report(output) {
      return `${output}\\report.md`;
    },
  },
};

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function redact(value) {
  return String(value)
    .replace(/\bck_(?:live|test)_[A-Za-z0-9._-]+\b/g, "ck_[REDACTED]")
    .replace(/\bsk-[A-Za-z0-9_-]{16,}\b/g, "sk-[REDACTED]")
    .replace(/(authorization\s*[:=]\s*bearer\s+)\S+/gi, "$1[REDACTED]")
    .slice(-4_000);
}

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
      redact(
        completed.stderr?.toString("utf8")
        || completed.stdout?.toString("utf8")
        || completed.error?.message
        || `${command} exited ${completed.status}`,
      ),
    );
  }
  return completed.stdout || Buffer.alloc(0);
}

function quotePython(value) {
  return JSON.stringify(String(value));
}

function quotePowerShell(value) {
  return `'${String(value).replaceAll("'", "''")}'`;
}

function executionProbe(platform, binary, output, hostLabel) {
  if (platform === "ubuntu") {
    return `import hashlib, json, pathlib, subprocess
binary = pathlib.Path(${quotePython(binary)})
output = pathlib.Path(${quotePython(output)})
binary.chmod(0o700)
completed = subprocess.run(
    [str(binary), "--out", str(output), "--host-label", ${quotePython(hostLabel)}, "--containment", "external"],
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True,
    timeout=180,
)
receipt_path = output / "receipt.json"
report_path = output / "report.md"
receipt = json.loads(receipt_path.read_text(encoding="utf-8")) if receipt_path.exists() else {}
payload = {
    "exit_code": completed.returncode,
    "receipt_present": receipt_path.exists(),
    "report_present": report_path.exists(),
    "receipt_sha256": hashlib.sha256(receipt_path.read_bytes()).hexdigest() if receipt_path.exists() else None,
    "report_sha256": hashlib.sha256(report_path.read_bytes()).hexdigest() if report_path.exists() else None,
    "receipt_status": receipt.get("status"),
    "canonical_sha256": receipt.get("canonical_sha256"),
}`;
  }
  return `$binary = ${quotePowerShell(binary)}
$output = ${quotePowerShell(output)}
$arguments = @(
  "--out", $output,
  "--host-label", ${quotePowerShell(hostLabel)},
  "--containment", "external"
)
$streamsBefore = @(
  Get-Item -LiteralPath $binary -Stream * -ErrorAction SilentlyContinue |
    ForEach-Object { $_.Stream }
)
Unblock-File -LiteralPath $binary -ErrorAction SilentlyContinue
$streamsAfter = @(
  Get-Item -LiteralPath $binary -Stream * -ErrorAction SilentlyContinue |
    ForEach-Object { $_.Stream }
)
$signature = Get-AuthenticodeSignature -LiteralPath $binary
$process = $null
$processError = $null
try {
  $process = Start-Process -FilePath $binary -ArgumentList $arguments -Wait -PassThru -WindowStyle Hidden
} catch {
  $processError = [string]$_.Exception.Message
}
$receiptPath = Join-Path $output "receipt.json"
$reportPath = Join-Path $output "report.md"
$receipt = if (Test-Path -LiteralPath $receiptPath) {
  Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
} else {
  $null
}
$payload = [ordered]@{
  binary_present = [bool](Test-Path -LiteralPath $binary)
  binary_length = if (Test-Path -LiteralPath $binary) {
    (Get-Item -LiteralPath $binary).Length
  } else { $null }
  streams_before_unblock = $streamsBefore
  streams_after_unblock = $streamsAfter
  signature_status = [string]$signature.Status
  process_error = $processError
  exit_code = if ($null -ne $process) { $process.ExitCode } else { $null }
  receipt_present = [bool](Test-Path -LiteralPath $receiptPath)
  report_present = [bool](Test-Path -LiteralPath $reportPath)
  receipt_sha256 = if (Test-Path -LiteralPath $receiptPath) {
    (Get-FileHash -LiteralPath $receiptPath -Algorithm SHA256).Hash.ToLowerInvariant()
  } else { $null }
  report_sha256 = if (Test-Path -LiteralPath $reportPath) {
    (Get-FileHash -LiteralPath $reportPath -Algorithm SHA256).Hash.ToLowerInvariant()
  } else { $null }
  receipt_status = if ($null -ne $receipt) { $receipt.status } else { $null }
  canonical_sha256 = if ($null -ne $receipt) { $receipt.canonical_sha256 } else { $null }
}`;
}

function windowsSecurityProbe(binary) {
  return `$ErrorActionPreference = "SilentlyContinue"
$binary = ${quotePowerShell(binary)}
$resource = "file:_" + $binary
$detections = @(
  Get-MpThreatDetection -ErrorAction SilentlyContinue |
    Where-Object { @($_.Resources) -contains $resource } |
    Select-Object InitialDetectionTime, LastThreatStatusChangeTime, ThreatID,
      ActionSuccess, CurrentThreatExecutionStatusID, Resources
)
$detectedThreatIds = @($detections | ForEach-Object { $_.ThreatID } | Sort-Object -Unique)
$threats = @(
  Get-MpThreat -ErrorAction SilentlyContinue |
    Where-Object { $detectedThreatIds -contains $_.ThreatID } |
    Select-Object ThreatID, ThreatName, SeverityID, CategoryID,
      DidThreatExecute, IsActive
)
$defender = Get-MpComputerStatus -ErrorAction SilentlyContinue
$binaryPresent = if ($detections.Count -gt 0) {
  $false
} else {
  [bool](Test-Path -LiteralPath $binary)
}
$item = if ($binaryPresent) { Get-Item -LiteralPath $binary } else { $null }
$signature = if ($binaryPresent) {
  Get-AuthenticodeSignature -LiteralPath $binary
} else { $null }
$payload = [ordered]@{
  binary_present = $binaryPresent
  binary_length = if ($binaryPresent) { $item.Length } else { $null }
  binary_sha256 = if ($binaryPresent) {
    (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant()
  } else { $null }
  binary_attributes = if ($binaryPresent) { [string]$item.Attributes } else { $null }
  binary_streams = if ($binaryPresent) {
    @(Get-Item -LiteralPath $binary -Stream * | ForEach-Object { $_.Stream })
  } else { @() }
  signature_status = if ($null -ne $signature) {
    [string]$signature.Status
  } else { $null }
  defender_service = if ($null -ne $defender) {
    [bool]$defender.AMServiceEnabled
  } else { $null }
  realtime_protection = if ($null -ne $defender) {
    [bool]$defender.RealTimeProtectionEnabled
  } else { $null }
  detections = $detections
  threats = $threats
}`;
}

function cleanupProbe(platform, binary, output) {
  if (platform === "ubuntu") {
    return `import pathlib, shutil
binary = pathlib.Path(${quotePython(binary)})
output = pathlib.Path(${quotePython(output)})
if binary.exists():
    binary.unlink()
if output.exists():
    shutil.rmtree(output)
payload = {
    "binary_removed": not binary.exists(),
    "output_removed": not output.exists(),
}`;
  }
  return `$binary = ${quotePowerShell(binary)}
$output = ${quotePowerShell(output)}
Remove-Item -LiteralPath $binary -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $output -Recurse -Force -ErrorAction SilentlyContinue
$payload = [ordered]@{
  binary_removed = -not (Test-Path -LiteralPath $binary)
  output_removed = -not (Test-Path -LiteralPath $output)
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

function enterpriseEvidence(receipt) {
  return receipt.cases?.find((entry) => entry.id === "enterprise_multi_machine_scale")?.evidence;
}

function qualify({ platform, binaryPath, outputDir, referencePath }) {
  const guest = GUESTS[platform];
  if (!guest) throw new Error("--platform must be ubuntu or windows");
  const binary = readFileSync(binaryPath);
  const reference = JSON.parse(readFileSync(referencePath, "utf8"));
  const marker = randomBytes(12).toString("hex");
  const guestBinary = guest.binary(marker);
  const guestOutput = guest.output(marker);
  const hostLabel = `utm_${platform}_current`;
  const validation = {
    schema_version: 2,
    status: "failed",
    platform,
    vm_name: guest.vm_name,
    marker,
    binary_sha256: sha256(binary),
    reference: {
      file: referencePath,
      canonical_sha256: reference.canonical_sha256,
      enterprise_semantic_sha256: enterpriseEvidence(reference)?.semantic_sha256,
    },
    transfer: {
      pushed: false,
      read_back_matches: false,
      read_back_sha256: null,
      read_back_length: null,
    },
    execution: null,
    security: null,
    cleanup: null,
    comparison: null,
    error: null,
  };
  if (existsSync(outputDir)) {
    throw new Error(`refusing to overwrite Scout UTM output ${outputDir}`);
  }
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  try {
    runBytes("utmctl", ["file", "push", guest.vm_name, guestBinary], {
      input: binary,
    });
    validation.transfer.pushed = true;
    const readBack = runBytes("utmctl", ["file", "pull", guest.vm_name, guestBinary]);
    validation.transfer.read_back_sha256 = sha256(readBack);
    validation.transfer.read_back_length = readBack.length;
    validation.transfer.read_back_matches = (
      validation.transfer.read_back_sha256 === sha256(binary)
    );
    if (!validation.transfer.read_back_matches) {
      if (platform === "windows") {
        validation.security = executeGuestJson({
          platform,
          vmName: guest.vm_name,
          state: "started",
          probeSource: windowsSecurityProbe(guestBinary),
          run,
          executionAttempts: 1,
        });
      }
      throw new Error("guest binary read-back SHA-256 mismatch");
    }

    const execution = executeGuestJson({
      platform,
      vmName: guest.vm_name,
      state: "started",
      probeSource: executionProbe(platform, guestBinary, guestOutput, hostLabel),
      run,
      timeoutMs: 180_000,
      pollAttempts: 240,
      pollDelayMs: 500,
      executionAttempts: 1,
    });
    validation.execution = execution;
    if (
      !execution.ok
      || execution.data?.exit_code !== 0
      || execution.data?.receipt_present !== true
      || execution.data?.report_present !== true
      || execution.data?.receipt_status !== "passed"
    ) {
      if (platform === "windows") {
        validation.security = executeGuestJson({
          platform,
          vmName: guest.vm_name,
          state: "started",
          probeSource: windowsSecurityProbe(guestBinary),
          run,
          executionAttempts: 1,
        });
      }
      throw new Error(execution.error || "guest benchmark did not produce a passing receipt");
    }
    if (platform === "windows") {
      validation.security = executeGuestJson({
        platform,
        vmName: guest.vm_name,
        state: "started",
        probeSource: windowsSecurityProbe(guestBinary),
        run,
        executionAttempts: 1,
      });
      if (
        !validation.security.ok
        || validation.security.data?.binary_present !== true
        || validation.security.data?.binary_sha256 !== sha256(binary)
        || validation.security.data?.defender_service !== true
        || validation.security.data?.realtime_protection !== true
        || validation.security.data?.detections?.length !== 0
      ) {
        throw new Error(
          validation.security.error
          || "Windows endpoint-security controls did not remain clean and enforced",
        );
      }
    }

    const receiptBytes = runBytes(
      "utmctl",
      ["file", "pull", guest.vm_name, guest.receipt(guestOutput)],
    );
    const reportBytes = runBytes(
      "utmctl",
      ["file", "pull", guest.vm_name, guest.report(guestOutput)],
    );
    if (
      sha256(receiptBytes) !== execution.data.receipt_sha256
      || sha256(reportBytes) !== execution.data.report_sha256
    ) {
      throw new Error("guest receipt changed while crossing the UTM boundary");
    }
    const receipt = JSON.parse(receiptBytes.toString("utf8").replace(/^\uFEFF/, ""));
    writeFileSync(path.join(outputDir, "receipt.json"), receiptBytes, { mode: 0o600 });
    writeFileSync(path.join(outputDir, "report.md"), reportBytes, { mode: 0o600 });

    const expected = enterpriseEvidence(reference);
    const actual = enterpriseEvidence(receipt);
    validation.comparison = {
      canonical: receipt.canonical_sha256 === reference.canonical_sha256,
      enterprise_semantic: actual?.semantic_sha256 === expected?.semantic_sha256,
      event_root: actual?.event_root === expected?.event_root,
      graph_digest: actual?.graph_digest === expected?.graph_digest,
      indexed_event_root: actual?.indexed_event_root === expected?.indexed_event_root,
      indexed_graph_digest: actual?.indexed_graph_digest === expected?.indexed_graph_digest,
    };
    validation.status = Object.values(validation.comparison).every(Boolean)
      ? "passed"
      : "failed";
  } catch (error) {
    validation.error = redact(error.message || error);
  } finally {
    const cleanup = executeGuestJson({
      platform,
      vmName: guest.vm_name,
      state: "started",
      probeSource: cleanupProbe(platform, guestBinary, guestOutput),
      run,
      timeoutMs: 180_000,
      pollAttempts: 60,
      pollDelayMs: 250,
      executionAttempts: 1,
    });
    validation.cleanup = cleanup;
    if (
      !cleanup.ok
      || cleanup.data?.binary_removed !== true
      || cleanup.data?.output_removed !== true
    ) {
      validation.status = "failed";
      validation.error ||= cleanup.error || "guest scratch cleanup was not proven";
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
  console.log(`Scout marker-authenticated UTM qualification

Usage:
  node harness/scout-utm-qualify.mjs
    --platform ubuntu|windows
    --binary PATH
    --reference RECEIPT_JSON
    --out NEW_DIRECTORY`);
  process.exit(0);
}

const platform = valueArg(args, "--platform");
const binaryPath = path.resolve(valueArg(args, "--binary") || "");
const referencePath = path.resolve(valueArg(args, "--reference") || "");
const outputDir = path.resolve(valueArg(args, "--out") || "");
if (!platform || !binaryPath || !referencePath || !outputDir) {
  throw new Error("--platform, --binary, --reference, and --out are required");
}
const result = qualify({ platform, binaryPath, outputDir, referencePath });
console.log(`validation=${path.join(outputDir, "validation.json")}`);
console.log(`status=${result.status}`);
if (result.status !== "passed") process.exitCode = 1;
