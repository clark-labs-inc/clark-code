#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  accessSync,
  chmodSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

import { captureUtmWindowObservation } from "./utm-window-observation.mjs";
import {
  clickInlineSetupExpression,
  consentToSandboxSetup,
  firstRunObservationExpression,
  installedBoundaryProbe,
  postSetupObservationExpression,
  startVisibleConsoleMonitor,
  stopVisibleConsoleMonitor,
  waitForPostSetup,
  validateWindowsInstallReceipt,
} from "./utm-windows-release-journey.mjs";
import {
  configureWebViewDebugPolicy,
  evaluateWindowsClarkWebView,
  launchWindowsClarkCode,
  qaStorageExpression,
  windowsGuestProbe,
} from "./utm-windows-webview.mjs";
import {
  validateWindowsUpdateCandidateReceipt,
  verifyWindowsUpdateCandidateEndpoint,
} from "./windows-update-candidate.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const VM_NAME =
  process.env.CLARK_WINDOWS_QA_VM_NAME || "Clark QA - Windows 11 ARM";
const QMP_PORT = 47_111;
const QA_ROOT = String.raw`C:\Users\home\ClarkCodeQA`;
const INSTALL_ROOT = String.raw`C:\Users\home\AppData\Local\Clark Code`;
const GUEST_SEED = String.raw`C:\Users\Public\ClarkCode-update-seed.exe`;
export const WINDOWS_UPDATE_ASSERTIONS = [
  "immutable_candidate_update_channel",
  "signed_update_seed_installed",
  "trusted_uac_consent_observed",
  "seed_inline_sandbox_setup",
  "signed_update_offered",
  "installed_update_version",
  "installed_update_signature",
  "sandbox_persisted_across_update",
  "updated_client_relaunched",
  "no_visible_console_windows_during_update",
];

function sleep(milliseconds) {
  if (milliseconds > 0) {
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);
  }
}

function prepareOutput(outputDir) {
  try {
    accessSync(outputDir);
    throw new Error(`refusing to overwrite Windows update journey output ${outputDir}`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  chmodSync(outputDir, 0o700);
}

function run(command, args, options = {}) {
  const completed = spawnSync(command, args, {
    cwd: repoDir,
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

function assertion(id, passed, detail) {
  return { id, status: passed ? "passed" : "failed", detail };
}

export function validateWindowsUpdateJourneyReceipt(receipt, expectedRevision) {
  if (
    !/^[0-9a-f]{40}$/.test(expectedRevision || "")
    || receipt?.receipt_type !== "clark_code_windows_installed_update"
    || receipt?.status !== "passed"
    || receipt?.source_revision !== expectedRevision
    || receipt?.required_user_vm_actions !== 0
    || receipt?.human_input_observed !== false
    || receipt?.paid_calls_made !== false
  ) {
    throw new Error("Windows installed-update receipt is missing, stale, or malformed");
  }
  validateWindowsInstallReceipt({
    receipt_type: "clark_code_windows_release_install",
    status: "passed",
    required_user_vm_actions: 0,
    source_revision: receipt.source_revision,
    release_candidate: receipt.release_candidate,
  });
  const candidate = validateWindowsUpdateCandidateReceipt(
    receipt.update_candidate,
    expectedRevision,
  );
  const assertions = new Map(
    (receipt.assertions || []).map((item) => [item.id, item.status]),
  );
  if (
    receipt.assertions?.length !== WINDOWS_UPDATE_ASSERTIONS.length
    || WINDOWS_UPDATE_ASSERTIONS.some((id) => assertions.get(id) !== "passed")
    || receipt.seed?.version !== candidate.seed_version
    || !/^[0-9a-f]{64}$/.test(receipt.seed?.sha256 || "")
    || receipt.seed?.install?.status !== "passed"
    || receipt.update_endpoint?.url !== candidate.endpoint
    || receipt.update_endpoint?.sha256 !== candidate.manifest_sha256
    || receipt.update_endpoint?.version !== candidate.version
    || receipt.final_boundary?.installed_version !== candidate.version
    || receipt.final_boundary?.signature_status !== "Valid"
    || receipt.final_boundary?.signer_thumbprint
      !== receipt.release_candidate.signer_thumbprint
    || receipt.final_boundary?.sandbox_marker_exists !== true
    || receipt.final_boundary?.sandbox_state_outside_install_root !== true
    || receipt.final_boundary?.visible_console_processes?.length !== 0
    || receipt.console_monitor?.observations?.length !== 0
    || receipt.uac_observation?.gui_visible !== true
    || receipt.uac_observation?.capture_transport !== "macos_window_id"
    || !/^[0-9a-f]{64}$/.test(receipt.uac_observation?.screenshot_sha256 || "")
    || receipt.uac_boundary?.uac_consent_process_present !== true
    || receipt.updated_webview?.value?.sandbox?.state !== "enforced"
    || !receipt.updated_webview?.value?.text?.includes(
      `Updated to v${candidate.version}`,
    )
  ) {
    throw new Error("Windows installed-update receipt lacks exact passing evidence");
  }
  return receipt;
}

export function seedInstallProbe({
  seedVersion,
  expectedSignerThumbprint,
  sourceRevision,
}) {
  if (!/^\d+\.\d+\.\d+$/.test(seedVersion)) throw new Error("invalid seed version");
  if (!/^[0-9A-Fa-f]{40}$/.test(expectedSignerThumbprint)) {
    throw new Error("invalid seed signer thumbprint");
  }
  if (!/^[0-9a-f]{40}$/.test(sourceRevision)) {
    throw new Error("invalid source revision");
  }
  return String.raw`
$installer = '${GUEST_SEED}'
$installRoot = '${INSTALL_ROOT}'
$executable = Join-Path $installRoot 'clark-desktop.exe'
$sandboxRoot = Join-Path $env:LOCALAPPDATA 'Clark\Code\sandbox'
$sandboxMarker = Join-Path $sandboxRoot 'setup-marker-v1.json'
$expectedVersion = '${seedVersion}'
$expectedSigner = '${expectedSignerThumbprint}'
$sourceRevision = '${sourceRevision}'

$seedSignature = Get-AuthenticodeSignature -LiteralPath $installer
if (
  $seedSignature.Status -ne [Management.Automation.SignatureStatus]::Valid -or
  $seedSignature.SignerCertificate.Thumbprint -ne $expectedSigner
) {
  throw "updater seed does not share the release Authenticode identity"
}
Get-Process -Name 'clark-desktop' -ErrorAction SilentlyContinue |
  Stop-Process -Force -ErrorAction SilentlyContinue
$existingUninstaller = Join-Path $installRoot 'uninstall.exe'
if (Test-Path -LiteralPath $existingUninstaller) {
  $uninstall = Start-Process -FilePath $existingUninstaller -ArgumentList @('/S') -Wait -PassThru -WindowStyle Hidden
  if ($uninstall.ExitCode -ne 0) {
    throw "existing Clark Code uninstall failed with exit code $($uninstall.ExitCode)"
  }
}
if (Test-Path -LiteralPath $installRoot) {
  Remove-Item -LiteralPath $installRoot -Recurse -Force
}
if (Test-Path -LiteralPath $sandboxRoot) {
  Remove-Item -LiteralPath $sandboxRoot -Recurse -Force
}
$install = Start-Process -FilePath $installer -ArgumentList @('/S', ('/D=' + $installRoot)) -Wait -PassThru -WindowStyle Hidden
if ($install.ExitCode -ne 0) {
  throw "updater seed install failed with exit code $($install.ExitCode)"
}
$installedVersion = (Get-Item -LiteralPath $executable).VersionInfo.ProductVersion
$appSignature = Get-AuthenticodeSignature -LiteralPath $executable
if (
  $installedVersion -ne $expectedVersion -or
  $appSignature.Status -ne [Management.Automation.SignatureStatus]::Valid -or
  $appSignature.SignerCertificate.Thumbprint -ne $expectedSigner -or
  (Test-Path -LiteralPath $sandboxMarker)
) {
  throw "updater seed installation identity or clean state is invalid"
}
$payload = [ordered]@{
  status = 'passed'
  source_revision = $sourceRevision
  seed_version = $installedVersion
  signer_thumbprint = $appSignature.SignerCertificate.Thumbprint
  fresh_sandbox_state = -not (Test-Path -LiteralPath $sandboxMarker)
}
`;
}

function updateBoundaryProbe() {
  return windowsGuestProbe({ probeSource: String.raw`
$installRoot = '${INSTALL_ROOT}'
$executable = Join-Path $installRoot 'clark-desktop.exe'
$sandboxMarker = Join-Path $env:LOCALAPPDATA 'Clark\Code\sandbox\setup-marker-v1.json'
$sandboxStateOutsideInstallRoot = -not $sandboxMarker.StartsWith(
  $installRoot + '\',
  [StringComparison]::OrdinalIgnoreCase
)
$signature = Get-AuthenticodeSignature -LiteralPath $executable
$visibleConsoles = @(
  Get-Process -ErrorAction SilentlyContinue |
    Where-Object {
      $_.MainWindowHandle -ne 0 -and
      $_.ProcessName -in @('cmd', 'conhost', 'powershell', 'pwsh', 'WindowsTerminal')
    } |
    Select-Object ProcessName, Id, MainWindowTitle
)
$payload = [ordered]@{
  installed_version = (Get-Item -LiteralPath $executable).VersionInfo.ProductVersion
  signature_status = $signature.Status.ToString()
  signer_thumbprint = if ($signature.SignerCertificate) {
    $signature.SignerCertificate.Thumbprint
  } else {
    ''
  }
  sandbox_marker_exists = Test-Path -LiteralPath $sandboxMarker -PathType Leaf
  sandbox_state_outside_install_root = $sandboxStateOutsideInstallRoot
  visible_console_processes = $visibleConsoles
}
` });
}

function updateReadyExpression(expectedVersion) {
  return `(() => {
    const button = [...document.querySelectorAll("button")].find((candidate) =>
      (candidate.getAttribute("aria-label") || "").includes(
        ${JSON.stringify(`Ready to update Clark Code to ${expectedVersion}`)}
      )
      || (candidate.innerText || "").includes(
        ${JSON.stringify(`Restart to update to ${expectedVersion}`)}
      )
    );
    return {
      ready: Boolean(button),
      text: document.body?.innerText || "",
      aria_label: button?.getAttribute("aria-label") || ""
    };
  })()`;
}

function clickUpdateExpression(expectedVersion) {
  return `(() => {
    const button = [...document.querySelectorAll("button")].find((candidate) =>
      (candidate.getAttribute("aria-label") || "").includes(
        ${JSON.stringify(`Ready to update Clark Code to ${expectedVersion}`)}
      )
      || (candidate.innerText || "").includes(
        ${JSON.stringify(`Restart to update to ${expectedVersion}`)}
      )
    );
    if (!button) return { clicked: false };
    button.click();
    return { clicked: true };
  })()`;
}

function waitForWebView(expression, { attempts = 120, delayMs = 1_000 } = {}) {
  let lastError = null;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    if (attempt > 0) sleep(delayMs);
    try {
      return {
        value: evaluateWindowsClarkWebView({ expression }),
        attempts: attempt + 1,
      };
    } catch (error) {
      lastError = error;
    }
  }
  throw new Error(`Windows WebView did not return: ${lastError}`);
}

function waitForInstalledVersion(expectedVersion) {
  let observed = null;
  let lastError = null;
  for (let attempt = 0; attempt < 180; attempt += 1) {
    if (attempt > 0) sleep(1_000);
    try {
      observed = updateBoundaryProbe();
      if (observed.installed_version === expectedVersion) {
        return { ...observed, attempts: attempt + 1 };
      }
    } catch (error) {
      lastError = error;
    }
  }
  throw new Error(
    `Windows update did not install ${expectedVersion}: ${lastError || JSON.stringify(observed)}`,
  );
}

function waitForUpdateReady(expectedVersion) {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    if (attempt > 0) sleep(1_000);
    const observation = evaluateWindowsClarkWebView({
      expression: updateReadyExpression(expectedVersion),
    });
    if (observation?.ready) return { ...observation, attempts: attempt + 1 };
  }
  return { ready: false, attempts: 120 };
}

export async function runWindowsUpdateJourney({
  seedInstallerPath,
  installReceiptPath,
  updateCandidateReceiptPath,
  outputDir,
}) {
  prepareOutput(outputDir);
  const install = validateWindowsInstallReceipt(
    JSON.parse(readFileSync(installReceiptPath, "utf8")),
  );
  const updateCandidate = validateWindowsUpdateCandidateReceipt(
    JSON.parse(readFileSync(updateCandidateReceiptPath, "utf8")),
    install.source_revision,
  );
  if (
    updateCandidate.version !== install.release_candidate.expected_version
    || updateCandidate.artifact_url !== install.release_candidate.immutable_url
  ) {
    throw new Error("update candidate does not identify the directly verified installer");
  }
  const endpointEvidence =
    await verifyWindowsUpdateCandidateEndpoint(updateCandidate);
  const seedBytes = readFileSync(seedInstallerPath);
  const seedSha256 = createHash("sha256").update(seedBytes).digest("hex");
  const pushed = run("utmctl", ["file", "push", VM_NAME, GUEST_SEED], {
    binary_output: true,
    input: seedBytes,
  });
  if (!pushed.ok) throw new Error(`could not push updater seed: ${pushed.stderr}`);
  const seedInstall = windowsGuestProbe({
    probeSource: seedInstallProbe({
      seedVersion: updateCandidate.seed_version,
      expectedSignerThumbprint: install.release_candidate.signer_thumbprint,
      sourceRevision: install.source_revision,
    }),
  });
  const assertions = [
    assertion(
      "immutable_candidate_update_channel",
      updateCandidate.endpoint.endsWith(
        `/releases/${updateCandidate.tag}/windows-update.json`,
      ),
      updateCandidate.manifest_sha256,
    ),
    assertion(
      "signed_update_seed_installed",
      seedInstall.status === "passed"
        && seedInstall.seed_version === updateCandidate.seed_version
        && seedInstall.signer_thumbprint
          === install.release_candidate.signer_thumbprint,
      seedSha256,
    ),
  ];
  let firstRun = null;
  let postSetup = null;
  let updateReady = null;
  let updateClick = null;
  let updatedWebView = null;
  let finalBoundary = null;
  let uacObservation = null;
  let uacBoundary = null;
  let cleanup = null;
  let consoleMonitor = null;
  try {
    configureWebViewDebugPolicy({ enabled: true });
    await launchWindowsClarkCode({ qmpPort: QMP_PORT });
    evaluateWindowsClarkWebView({
      expression: qaStorageExpression({ cwd: QA_ROOT }),
    });
    evaluateWindowsClarkWebView({
      expression: "location.reload(); ({ reloading: true })",
    });
    sleep(4_000);
    firstRun = evaluateWindowsClarkWebView({
      expression: firstRunObservationExpression(),
    });
    startVisibleConsoleMonitor("installed-update");
    const setupClick = evaluateWindowsClarkWebView({
      expression: clickInlineSetupExpression(),
    });
    sleep(1_500);
    uacObservation = await captureUtmWindowObservation({
      platform: "windows",
      vmName: VM_NAME,
      qmpPort: QMP_PORT,
      outputDir: path.join(outputDir, "uac-evidence"),
    });
    uacBoundary = installedBoundaryProbe();
    await consentToSandboxSetup();
    postSetup = waitForPostSetup();
    assertions.push(
      assertion(
        "trusted_uac_consent_observed",
        uacObservation.gui_visible === true
          && uacObservation.capture_transport === "macos_window_id"
          && /^[0-9a-f]{64}$/.test(uacObservation.screenshot_sha256 || "")
          && uacBoundary.uac_consent_process_present === true,
        "fresh exact-VM capture and the Windows consent process prove the seed UAC boundary",
      ),
      assertion(
        "seed_inline_sandbox_setup",
        firstRun.inline_setup_visible === true
          && firstRun.send_disabled === true
          && setupClick.clicked === true
          && postSetup.sandbox?.state === "enforced",
        "sandbox enrolled through the installed seed client",
      ),
    );

    updateReady = waitForUpdateReady(updateCandidate.version);
    updateClick = evaluateWindowsClarkWebView({
      expression: clickUpdateExpression(updateCandidate.version),
    });
    assertions.push(
      assertion(
        "signed_update_offered",
        updateReady.ready === true && updateClick.clicked === true,
        updateReady.aria_label || "update action was not available",
      ),
    );
    finalBoundary = waitForInstalledVersion(updateCandidate.version);
    updatedWebView = waitForWebView(`(async () => ({
      text: document.body?.innerText || "",
      sandbox: await window.__TAURI_INTERNALS__.invoke(
        "local_sandbox_status",
        { cwd: ${JSON.stringify(QA_ROOT)} }
      )
    }))()`);
    consoleMonitor = stopVisibleConsoleMonitor("installed-update");
    assertions.push(
      assertion(
        "installed_update_version",
        finalBoundary.installed_version === updateCandidate.version,
        `${finalBoundary.installed_version} from ${updateCandidate.seed_version}`,
      ),
      assertion(
        "installed_update_signature",
        finalBoundary.signature_status === "Valid"
          && finalBoundary.signer_thumbprint
            === install.release_candidate.signer_thumbprint,
        finalBoundary.signer_thumbprint,
      ),
      assertion(
        "sandbox_persisted_across_update",
        finalBoundary.sandbox_marker_exists === true
          && finalBoundary.sandbox_state_outside_install_root === true
          && updatedWebView.value?.sandbox?.state === "enforced",
        "sandbox marker and enforcement survived replacement and relaunch",
      ),
      assertion(
        "updated_client_relaunched",
        updatedWebView.value?.text?.includes(
          `Updated to v${updateCandidate.version}`,
        ) === true,
        "updated client displayed its one-time relaunch confirmation",
      ),
      assertion(
        "no_visible_console_windows_during_update",
        finalBoundary.visible_console_processes?.length === 0
          && consoleMonitor.observations?.length === 0,
        "75ms monitor saw no cmd, PowerShell, conhost, or Terminal window during setup, update, or relaunch",
      ),
    );
  } finally {
    if (!consoleMonitor) {
      try {
        consoleMonitor = stopVisibleConsoleMonitor("installed-update");
      } catch {
        // The monitor may not have started if the journey failed earlier.
      }
    }
    try {
      cleanup = configureWebViewDebugPolicy({ enabled: false });
    } catch (error) {
      cleanup = { policy_state_matches: false, error: String(error) };
    }
  }
  const receipt = {
    schema_version: 1,
    receipt_type: "clark_code_windows_installed_update",
    status: assertions.length === WINDOWS_UPDATE_ASSERTIONS.length
      && assertions.every((item) => item.status === "passed")
      ? "passed"
      : "failed",
    generated_at: new Date().toISOString(),
    source_revision: install.source_revision,
    platform: "windows",
    virtualization: "utm",
    vm_name: VM_NAME,
    required_user_vm_actions: 0,
    human_input_observed: false,
    release_candidate: install.release_candidate,
    update_candidate: updateCandidate,
    update_endpoint: endpointEvidence,
    seed: {
      version: updateCandidate.seed_version,
      sha256: seedSha256,
      install: seedInstall,
    },
    assertions,
    first_run: firstRun,
    post_setup: postSetup,
    update_ready: updateReady,
    update_click: updateClick,
    updated_webview: updatedWebView,
    final_boundary: finalBoundary,
    console_monitor: consoleMonitor,
    uac_observation: uacObservation,
    uac_boundary: uacBoundary,
    cleanup,
    paid_calls_made: false,
  };
  const receiptPath = path.join(outputDir, "receipt.json");
  writeFileSync(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, {
    mode: 0o600,
  });
  chmodSync(receiptPath, 0o600);
  return { receipt, receiptPath };
}

function valueArg(args, name) {
  const inline = args.find((arg) => arg.startsWith(`${name}=`));
  if (inline) return inline.slice(name.length + 1);
  const index = args.indexOf(name);
  if (index < 0 || !args[index + 1] || args[index + 1].startsWith("--")) {
    throw new Error(`${name} requires a value`);
  }
  return args[index + 1];
}

async function main() {
  const args = process.argv.slice(2);
  const result = await runWindowsUpdateJourney({
    seedInstallerPath: path.resolve(valueArg(args, "--seed-installer")),
    installReceiptPath: path.resolve(valueArg(args, "--install-receipt")),
    updateCandidateReceiptPath: path.resolve(
      valueArg(args, "--update-candidate-receipt"),
    ),
    outputDir: path.resolve(valueArg(args, "--out")),
  });
  process.stdout.write(`${JSON.stringify({
    status: result.receipt.status,
    receipt: result.receiptPath,
    failed_assertions: result.receipt.assertions
      .filter((item) => item.status !== "passed")
      .map((item) => item.id),
  }, null, 2)}\n`);
  if (result.receipt.status !== "passed") process.exitCode = 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
