#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

import { executeGuestJson } from "./utm-guest-channel.mjs";
import { validateReleaseCandidateDownload } from "./download-release-candidate.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const WINDOWS_VM =
  process.env.CLARK_WINDOWS_QA_VM_NAME || "Clark QA - Windows 11 ARM";
const GUEST_INSTALLER = String.raw`C:\Users\Public\ClarkCode-release-candidate.exe`;
const INSTALL_ROOT = String.raw`C:\Users\home\AppData\Local\Clark Code`;

function run(command, args, options = {}) {
  const completed = spawnSync(command, args, {
    cwd: options.cwd || repoDir,
    env: options.env || process.env,
    encoding: options.binary_output ? null : "utf8",
    input: options.input,
    timeout: options.timeout_ms ?? 300_000,
    maxBuffer: 16 * 1024 * 1024,
  });
  return {
    ok: completed.status === 0,
    exit_code: completed.status,
    stdout: completed.stdout || (options.binary_output ? Buffer.alloc(0) : ""),
    stderr: completed.stderr || completed.error?.message || "",
  };
}

function exactVersion(value) {
  if (!/^\d+\.\d+\.\d+$/.test(value)) {
    throw new Error(`invalid expected version ${JSON.stringify(value)}`);
  }
  return value;
}

function exactSha256(value, label) {
  if (!/^[0-9a-f]{64}$/.test(value)) {
    throw new Error(`${label} must be a lowercase SHA-256 digest`);
  }
  return value;
}

function exactRevision(value) {
  if (!/^[0-9a-f]{40}$/.test(value)) {
    throw new Error("source revision must be a clean 40-character Git revision");
  }
  return value;
}

export function windowsReleaseInstallProbe({
  expectedVersion,
  expectedSha256,
  expectedSignerSubject,
  expectedSignerThumbprint,
  sourceRevision,
}) {
  exactVersion(expectedVersion);
  exactSha256(expectedSha256, "expected installer digest");
  if (
    typeof expectedSignerSubject !== "string"
    || expectedSignerSubject.trim().length === 0
    || /[\r\n]/.test(expectedSignerSubject)
  ) {
    throw new Error("expected signer subject is required");
  }
  if (!/^[0-9A-F]{40}$/.test(expectedSignerThumbprint)) {
    throw new Error("expected signer thumbprint must be 40 uppercase hexadecimal characters");
  }
  exactRevision(sourceRevision);
  return String.raw`
$installer = '${GUEST_INSTALLER}'
$installRoot = '${INSTALL_ROOT}'
$expectedVersion = '${expectedVersion}'
$expectedSha256 = '${expectedSha256}'
$expectedSignerSubject = '${expectedSignerSubject.replaceAll("'", "''")}'
$expectedSignerThumbprint = '${expectedSignerThumbprint}'
$sourceRevision = '${sourceRevision}'
$executable = Join-Path $installRoot 'clark-desktop.exe'
$sandboxRoot = Join-Path $env:LOCALAPPDATA 'Clark\Code\sandbox'
$sandboxMarker = Join-Path $sandboxRoot 'setup-marker-v1.json'
$sandboxStateOutsideInstallRoot = -not $sandboxMarker.StartsWith(
  $installRoot + '\',
  [StringComparison]::OrdinalIgnoreCase
)
if (-not $sandboxStateOutsideInstallRoot) {
  throw "sandbox state must be outside the replaceable NSIS install root"
}

$actualSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $installer).Hash.ToLower()
if ($actualSha256 -ne $expectedSha256) {
  throw "release-candidate installer SHA-256 mismatch"
}
$installerSignature = Get-AuthenticodeSignature -LiteralPath $installer
if ($installerSignature.Status -ne [Management.Automation.SignatureStatus]::Valid) {
  throw "release-candidate installer does not have a valid Authenticode signature"
}
$signerThumbprint = $installerSignature.SignerCertificate.Thumbprint
if (
  $installerSignature.SignerCertificate.Subject -ne $expectedSignerSubject -or
  $signerThumbprint -ne $expectedSignerThumbprint
) {
  throw "release-candidate installer does not match the Clark build signer"
}
$uacEnabled = (Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System' -Name EnableLUA).EnableLUA -eq 1
if (-not $uacEnabled) { throw "Windows UAC must be enabled for release verification" }

Get-Process -Name 'clark-desktop' -ErrorAction SilentlyContinue |
  ForEach-Object { throw "pristine release guest already has Clark Code running" }
$existingUninstaller = Join-Path $installRoot 'uninstall.exe'
$legacySandboxRoot = Join-Path $env:LOCALAPPDATA 'Clark Code\sandbox'
$uninstallRegistrations = @(
  @(
    'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
    'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*'
  ) | ForEach-Object {
    Get-ItemProperty -Path $_ -ErrorAction SilentlyContinue
  } | Where-Object { $_.DisplayName -like '*Clark Code*' }
  Get-ChildItem 'Registry::HKEY_USERS' -ErrorAction SilentlyContinue |
    ForEach-Object {
      Get-ItemProperty -Path (
        'Registry::' + $_.Name + '\Software\Microsoft\Windows\CurrentVersion\Uninstall\*'
      ) -ErrorAction SilentlyContinue
    } | Where-Object { $_.DisplayName -like '*Clark Code*' }
)
$offlineIdentity = Get-CimInstance Win32_UserAccount -Filter "LocalAccount=True AND Name='ClarkSandboxOffline'" -ErrorAction SilentlyContinue
$sandboxFirewallRules = @(
  Get-NetFirewallRule -ErrorAction SilentlyContinue |
    Where-Object {
      $_.Name -in @(
        'clark_sandbox_offline_block_outbound',
        'clark_sandbox_offline_block_loopback',
        'clark_sandbox_offline_block_loopback_tcp',
        'clark_sandbox_offline_block_loopback_udp'
      )
    }
)
if (
  (Test-Path -LiteralPath $existingUninstaller) -or
  (Test-Path -LiteralPath $installRoot) -or
  (Test-Path -LiteralPath $sandboxRoot) -or
  (Test-Path -LiteralPath $legacySandboxRoot) -or
  $uninstallRegistrations.Count -ne 0 -or
  $null -ne $offlineIdentity -or
  $sandboxFirewallRules.Count -ne 0
) {
  throw "release candidate requires the verified pristine Windows clone"
}
$freshInstallState = -not (Test-Path -LiteralPath $executable) -and $uninstallRegistrations.Count -eq 0
$freshSandboxState = -not (Test-Path -LiteralPath $sandboxMarker) -and -not (Test-Path -LiteralPath $legacySandboxRoot) -and $null -eq $offlineIdentity -and $sandboxFirewallRules.Count -eq 0
if (-not $freshInstallState -or -not $freshSandboxState) {
  throw "could not establish a clean Clark Code installation and sandbox state"
}

$install = Start-Process -FilePath $installer -ArgumentList @('/S', ('/D=' + $installRoot)) -Wait -PassThru -WindowStyle Hidden
if ($install.ExitCode -ne 0) {
  throw "release-candidate install failed with exit code $($install.ExitCode)"
}
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
  throw "release-candidate executable was not installed"
}
$installedVersion = (Get-Item -LiteralPath $executable).VersionInfo.ProductVersion
if ($installedVersion -ne $expectedVersion) {
  throw "installed version $installedVersion does not match expected $expectedVersion"
}
$appSignature = Get-AuthenticodeSignature -LiteralPath $executable
if (
  $appSignature.Status -ne [Management.Automation.SignatureStatus]::Valid -or
  $appSignature.SignerCertificate.Subject -ne $expectedSignerSubject -or
  $appSignature.SignerCertificate.Thumbprint -ne $expectedSignerThumbprint
) {
  throw "installed app does not share the valid installer Authenticode identity"
}
if (Test-Path -LiteralPath $sandboxMarker) {
  throw "release installer unexpectedly shipped an enrolled sandbox marker"
}

$payload = [ordered]@{
  receipt_type = 'clark_code_windows_release_install'
  status = 'passed'
  vm_name = '${WINDOWS_VM}'
  required_user_vm_actions = 0
  source_revision = $sourceRevision
  release_candidate = [ordered]@{
    installer_sha256 = $actualSha256
    expected_version = $expectedVersion
    installed_version = $installedVersion
    fresh_install = $freshInstallState
    fresh_sandbox_state = $freshSandboxState
    sandbox_state_outside_install_root = $sandboxStateOutsideInstallRoot
    uac_enabled = $uacEnabled
    executable = $executable
    signature_status = $installerSignature.Status.ToString()
    installed_signature_status = $appSignature.Status.ToString()
    signer_subject = $installerSignature.SignerCertificate.Subject
    signer_thumbprint = $signerThumbprint
  }
}
`;
}

export function installWindowsReleaseCandidate({
  candidateReceiptPath,
  expectedSourceRevision,
  outputDir,
}) {
  exactRevision(expectedSourceRevision);
  const candidateReceiptBytes = readFileSync(candidateReceiptPath);
  const candidate = validateReleaseCandidateDownload(
    JSON.parse(candidateReceiptBytes.toString("utf8")),
    expectedSourceRevision,
  );
  const installer = path.join(
    path.dirname(candidateReceiptPath),
    candidate.artifact.file,
  );
  const installerBytes = readFileSync(installer);
  const localSha256 = createHash("sha256").update(installerBytes).digest("hex");
  if (
    installerBytes.length !== candidate.artifact.size
    || localSha256 !== candidate.artifact.sha256
  ) {
    throw new Error("downloaded Windows candidate no longer matches its receipt");
  }
  if (existsSync(outputDir)) {
    throw new Error(`refusing to overwrite existing output directory ${outputDir}`);
  }
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  chmodSync(outputDir, 0o700);

  const pushed = run(
    "utmctl",
    ["file", "push", WINDOWS_VM, GUEST_INSTALLER],
    {
      binary_output: true,
      input: installerBytes,
      timeout_ms: 300_000,
    },
  );
  if (!pushed.ok) {
    throw new Error(`could not push release candidate to Windows QA VM: ${pushed.stderr}`);
  }
  const result = executeGuestJson({
    platform: "windows",
    vmName: WINDOWS_VM,
    state: "started",
    probeSource: windowsReleaseInstallProbe({
      expectedVersion: candidate.version,
      expectedSha256: candidate.artifact.sha256,
      expectedSignerSubject: candidate.signer_subject,
      expectedSignerThumbprint: candidate.signer_thumbprint,
      sourceRevision: candidate.source_revision,
    }),
    run,
    timeoutMs: 300_000,
    pollAttempts: 120,
    pollDelayMs: 1_000,
    executionAttempts: 1,
  });
  if (!result.ok) {
    throw new Error(`Windows release-candidate install failed: ${result.error}`);
  }
  Object.assign(result.data.release_candidate, {
    tag: candidate.tag,
    immutable_url: candidate.artifact.url,
    downloaded_size: candidate.artifact.size,
    source_revision: candidate.artifact.source_revision,
    expected_signer_subject: candidate.signer_subject,
    expected_signer_thumbprint: candidate.signer_thumbprint,
    build_receipt_sha256: candidate.build_receipt_sha256,
    download_receipt_sha256: createHash("sha256")
      .update(candidateReceiptBytes)
      .digest("hex"),
  });
  const receiptPath = path.join(outputDir, "receipt.json");
  writeFileSync(receiptPath, `${JSON.stringify(result.data, null, 2)}\n`, {
    encoding: "utf8",
    mode: 0o600,
  });
  chmodSync(receiptPath, 0o600);
  return { receiptPath, receipt: result.data };
}

function parseArgs(argv) {
  const parsed = {};
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (!argument.startsWith("--") || index + 1 >= argv.length) {
      throw new Error(`invalid argument ${JSON.stringify(argument)}`);
    }
    parsed[argument.slice(2)] = argv[index + 1];
    index += 1;
  }
  for (const required of [
    "candidate-receipt",
    "source-revision",
    "out",
  ]) {
    if (!parsed[required]) throw new Error(`missing --${required}`);
  }
  return parsed;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const result = installWindowsReleaseCandidate({
    candidateReceiptPath: path.resolve(args["candidate-receipt"]),
    expectedSourceRevision: args["source-revision"],
    outputDir: path.resolve(args.out),
  });
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    process.stderr.write(`${error.stack || error.message}\n`);
    process.exitCode = 1;
  });
}
