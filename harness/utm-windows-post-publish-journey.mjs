#!/usr/bin/env node

import {
  accessSync,
  chmodSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

import {
  validatePublicReleaseJourneyReceipt,
} from "./public-release-journey.mjs";
import {
  validateWindowsUpdateJourneyReceipt,
} from "./utm-windows-update-journey.mjs";
import {
  startVisibleConsoleMonitor,
  stopVisibleConsoleMonitor,
} from "./utm-windows-release-journey.mjs";
import {
  configureWebViewDebugPolicy,
  evaluateWindowsClarkWebView,
  launchWindowsClarkCode,
  windowsGuestProbe,
} from "./utm-windows-webview.mjs";

const QMP_PORT = 47_111;
const QA_ROOT = String.raw`C:\Users\home\ClarkCodeQA`;
const INSTALL_ROOT = String.raw`C:\Users\home\AppData\Local\Clark Code`;

function sleep(milliseconds) {
  if (milliseconds > 0) {
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);
  }
}

function prepareOutput(outputDir) {
  try {
    accessSync(outputDir);
    throw new Error(`refusing to overwrite post-publish journey output ${outputDir}`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  chmodSync(outputDir, 0o700);
}

function assertion(id, passed, detail) {
  return { id, status: passed ? "passed" : "failed", detail };
}

function installedBoundaryProbe() {
  return windowsGuestProbe({ probeSource: String.raw`
$installRoot = '${INSTALL_ROOT}'
$executable = Join-Path $installRoot 'clark-desktop.exe'
$sandboxMarker = Join-Path $env:LOCALAPPDATA 'Clark\Code\sandbox\setup-marker-v1.json'
$sandboxStateOutsideInstallRoot = -not $sandboxMarker.StartsWith(
  $installRoot + '\',
  [StringComparison]::OrdinalIgnoreCase
)
$signature = Get-AuthenticodeSignature -LiteralPath $executable
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
  visible_console_processes = @(
    Get-Process -ErrorAction SilentlyContinue |
      Where-Object {
        $_.MainWindowHandle -ne 0 -and
        $_.ProcessName -in @('cmd', 'conhost', 'powershell', 'pwsh', 'WindowsTerminal')
      } |
      Select-Object ProcessName, Id, MainWindowTitle
  )
}
` });
}

function reloadForProductionUpdateExpression() {
  return `(() => {
    location.reload();
    return { reloading: true };
  })()`;
}

function clickCheckForUpdatesExpression() {
  return `(() => {
    const button = [...document.querySelectorAll("button")]
      .find((candidate) => (candidate.innerText || "").trim() === "Check for updates");
    if (!button) return { clicked: false, text: document.body?.innerText || "" };
    button.click();
    return { clicked: true };
  })()`;
}

function waitForUpToDate() {
  let observed = null;
  for (let attempt = 0; attempt < 60; attempt += 1) {
    if (attempt > 0) sleep(1_000);
    observed = evaluateWindowsClarkWebView({
      expression: `(async () => ({
        text: document.body?.innerText || "",
        sandbox: await window.__TAURI_INTERNALS__.invoke(
          "local_sandbox_status",
          { cwd: ${JSON.stringify(QA_ROOT)} }
        )
      }))()`,
    });
    if (observed.text?.includes("You’re on the latest version.")) {
      return { ...observed, attempts: attempt + 1 };
    }
  }
  return { ...observed, attempts: 60 };
}

export async function runWindowsPostPublishJourney({
  updateReceiptPath,
  publicReceiptPath,
  sourceRevision,
  outputDir,
}) {
  prepareOutput(outputDir);
  const update = validateWindowsUpdateJourneyReceipt(
    JSON.parse(readFileSync(updateReceiptPath, "utf8")),
    sourceRevision,
  );
  const publicRelease = validatePublicReleaseJourneyReceipt(
    JSON.parse(readFileSync(publicReceiptPath, "utf8")),
    sourceRevision,
  );
  if (
    publicRelease.tag !== update.update_candidate.tag
    || publicRelease.version !== update.update_candidate.version
  ) {
    throw new Error("public channel does not identify the installed update");
  }
  let checkClick = null;
  let upToDate = null;
  let boundary = null;
  let consoleMonitor = null;
  let cleanup = null;
  const assertions = [];
  try {
    configureWebViewDebugPolicy({ enabled: true });
    await launchWindowsClarkCode({ qmpPort: QMP_PORT });
    evaluateWindowsClarkWebView({
      expression: reloadForProductionUpdateExpression(),
    });
    sleep(3_000);
    startVisibleConsoleMonitor("post-publish");
    checkClick = evaluateWindowsClarkWebView({
      expression: clickCheckForUpdatesExpression(),
    });
    upToDate = waitForUpToDate();
    boundary = installedBoundaryProbe();
    consoleMonitor = stopVisibleConsoleMonitor("post-publish");
    assertions.push(
      assertion(
        "production_updater_reports_current",
        checkClick.clicked === true
          && upToDate.text?.includes("You’re on the latest version.") === true
          && !upToDate.text?.includes("Couldn’t check for updates"),
        `${publicRelease.base_url}/latest/latest.json`,
      ),
      assertion(
        "post_publish_installed_identity",
        boundary.installed_version === publicRelease.version
          && boundary.signature_status === "Valid"
          && boundary.signer_thumbprint
            === update.release_candidate.signer_thumbprint,
        boundary.signer_thumbprint,
      ),
      assertion(
        "post_publish_sandbox_persistence",
        boundary.sandbox_marker_exists === true
          && boundary.sandbox_state_outside_install_root === true
          && upToDate.sandbox?.state === "enforced",
        "sandbox remains enforced after production-channel revalidation",
      ),
      assertion(
        "post_publish_no_visible_consoles",
        boundary.visible_console_processes?.length === 0
          && consoleMonitor.observations?.length === 0,
        "75ms monitor saw no shell or console window during production update check",
      ),
    );
  } finally {
    if (!consoleMonitor) {
      try {
        consoleMonitor = stopVisibleConsoleMonitor("post-publish");
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
    receipt_type: "clark_code_windows_post_publish",
    status: assertions.length === 4
      && assertions.every((item) => item.status === "passed")
      ? "passed"
      : "failed",
    generated_at: new Date().toISOString(),
    source_revision: sourceRevision,
    tag: publicRelease.tag,
    version: publicRelease.version,
    platform: "windows",
    virtualization: "utm",
    required_user_vm_actions: 0,
    human_input_observed: false,
    assertions,
    check_click: checkClick,
    up_to_date: upToDate,
    installed_boundary: boundary,
    console_monitor: consoleMonitor,
    update_receipt_sha256: createHash("sha256")
      .update(readFileSync(updateReceiptPath))
      .digest("hex"),
    public_receipt_sha256: createHash("sha256")
      .update(readFileSync(publicReceiptPath))
      .digest("hex"),
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
  const result = await runWindowsPostPublishJourney({
    updateReceiptPath: path.resolve(valueArg(args, "--update-receipt")),
    publicReceiptPath: path.resolve(valueArg(args, "--public-receipt")),
    sourceRevision: valueArg(args, "--source-revision"),
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
