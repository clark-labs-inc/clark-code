#!/usr/bin/env node

import {
  accessSync,
  chmodSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { Buffer } from "node:buffer";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

import { QmpClient } from "./utm-qmp.mjs";
import { captureUtmWindowObservation } from "./utm-window-observation.mjs";
import {
  configureWebViewDebugPolicy,
  evaluateWindowsClarkWebView,
  launchWindowsClarkCode,
  localWindowsQaRetainedAuth,
  qaLocalSettingsExpression,
  seedWindowsNativeCredentials,
  windowsGuestProbe,
} from "./utm-windows-webview.mjs";
import {
  NATIVE_CONTAINMENT_ASSERTIONS,
  validateNativeContainmentReceipt,
} from "./windows-native-containment.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const VM_NAME =
  process.env.CLARK_WINDOWS_QA_VM_NAME || "Clark QA - Windows 11 ARM";
const QMP_PORT = 47_111;
const QA_ROOT = String.raw`C:\Users\home\ClarkCodeQA`;
const FULL_ACCESS_WARNING =
  "Run directly on your machine without Clark’s command sandbox or action approvals";
const INLINE_SETUP_HEADING = "Enable the Windows command sandbox";
const CONSOLE_MONITOR_ROOT = String.raw`C:\Users\Public\ClarkConsoleMonitor`;
export const WINDOWS_FIRST_RUN_ASSERTIONS = [
  ...NATIVE_CONTAINMENT_ASSERTIONS,
  "ordinary_pty_and_child_processes_hidden",
  "candidate_installer_identity",
  "uac_enabled",
  "inline_sandbox_setup",
  "sandbox_state_outside_install_root",
  "full_access_labeled_unsandboxed",
  "signed_sandbox_helpers",
  "trusted_uac_consent_observed",
  "sandbox_enforced_after_setup",
  "no_visible_console_windows",
  "integrated_terminal_uses_hidden_conpty",
  "packaged_sandbox_command_execution",
  "sandbox_enforced_after_restart",
];

function sleep(milliseconds) {
  if (milliseconds > 0) {
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);
  }
}

function prepareOutput(outputDir) {
  try {
    accessSync(outputDir);
    throw new Error(`refusing to overwrite Windows release journey output ${outputDir}`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  chmodSync(outputDir, 0o700);
}

export function validateWindowsInstallReceipt(receipt) {
  const candidate = receipt?.release_candidate;
  if (
    receipt?.receipt_type !== "clark_code_windows_release_install"
    || receipt?.status !== "passed"
    || receipt?.required_user_vm_actions !== 0
    || !/^[0-9a-f]{40}$/.test(receipt?.source_revision || "")
    || !/^[0-9a-f]{64}$/.test(candidate?.installer_sha256 || "")
    || !/^\d+\.\d+\.\d+$/.test(candidate?.expected_version || "")
    || candidate?.installed_version !== candidate?.expected_version
    || candidate?.fresh_install !== true
    || candidate?.fresh_sandbox_state !== true
    || candidate?.sandbox_state_outside_install_root !== true
    || candidate?.uac_enabled !== true
    || candidate?.tag !== `v${candidate?.expected_version}`
    || candidate?.immutable_url
      !== `https://downloads.clarkchat.com/desktop/releases/${candidate?.tag}/ClarkCode_x64-setup.exe`
    || candidate?.downloaded_size <= 0
    || !/^[0-9a-f]{64}$/.test(candidate?.download_receipt_sha256 || "")
    || !/^[0-9a-f]{64}$/.test(candidate?.build_receipt_sha256 || "")
    || candidate?.source_revision !== receipt.source_revision
    || candidate?.signature_status !== "Valid"
    || candidate?.installed_signature_status !== "Valid"
    || typeof candidate?.signer_subject !== "string"
    || candidate.signer_subject.trim().length === 0
    || /[\r\n]/.test(candidate.signer_subject)
    || candidate?.signer_subject !== candidate?.expected_signer_subject
    || !/^[0-9A-F]{40}$/.test(candidate?.signer_thumbprint || "")
    || candidate?.signer_thumbprint !== candidate?.expected_signer_thumbprint
  ) {
    throw new Error("Windows release journey requires a passed, fresh, exact install receipt");
  }
  return receipt;
}

export function firstRunObservationExpression() {
  return `(async () => {
    const text = document.body?.innerText || "";
    const sandbox = await window.__TAURI_INTERNALS__.invoke(
      "local_sandbox_status",
      { cwd: ${JSON.stringify(QA_ROOT)} }
    );
    return {
      title: document.title,
      text,
      inline_setup_visible: text.includes(${JSON.stringify(INLINE_SETUP_HEADING)}),
      enable_button_count: [...document.querySelectorAll("button")]
        .filter((button) => (button.innerText || "").trim() === "Enable sandbox").length,
      send_disabled: Boolean(
        [...document.querySelectorAll("button")]
          .find((button) => (button.getAttribute("aria-label") || "").includes("Send"))
          ?.disabled
      ),
      sandbox
    };
  })()`;
}

export function fullAccessObservationExpression() {
  return `(async () => {
    const policy = [...document.querySelectorAll("button")]
      .find((button) => (button.innerText || "").includes("Approve for me"));
    policy?.click();
    await new Promise((resolve) => setTimeout(resolve, 150));
    const text = document.body?.innerText || "";
    return {
      full_access_visible: text.includes("Full access"),
      full_access_warning_visible: text.includes(${JSON.stringify(FULL_ACCESS_WARNING)}),
      text
    };
  })()`;
}

export function clickInlineSetupExpression() {
  return `(() => {
    const button = [...document.querySelectorAll("button")]
      .find((candidate) => (candidate.innerText || "").trim() === "Enable sandbox");
    if (!button) return { clicked: false };
    button.click();
    return { clicked: true };
  })()`;
}

export function postSetupObservationExpression() {
  return `(async () => {
    const text = document.body?.innerText || "";
    return {
      sandbox: await window.__TAURI_INTERNALS__.invoke(
        "local_sandbox_status",
        { cwd: ${JSON.stringify(QA_ROOT)} }
      ),
      inline_setup_visible: text.includes(${JSON.stringify(INLINE_SETUP_HEADING)}),
      setup_error_visible: text.includes("Sandbox setup is unavailable")
        || text.includes("sandbox setup did not become ready")
    };
  })()`;
}

export function waitForPostSetup({ attempts = 60, delayMs = 1_000 } = {}) {
  let observed = null;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    if (attempt > 0) sleep(delayMs);
    observed = evaluateWindowsClarkWebView({
      expression: postSetupObservationExpression(),
    });
    if (observed?.sandbox?.state === "enforced") {
      return { ...observed, attempts: attempt + 1 };
    }
  }
  return { ...observed, attempts };
}

export function installedBoundaryProbe() {
  return windowsGuestProbe({ probeSource: String.raw`
$helperRoot = 'C:\Users\home\AppData\Local\Clark Code\clark-resources\sandbox\windows'
$names = @('clark-command-runner.exe', 'clark-windows-sandbox-setup.exe')
$helpers = @(
  foreach ($name in $names) {
    $file = Join-Path $helperRoot $name
    $signature = Get-AuthenticodeSignature -LiteralPath $file
    [ordered]@{
      name = $name
      exists = Test-Path -LiteralPath $file -PathType Leaf
      signature_status = $signature.Status.ToString()
      signer_thumbprint = if ($signature.SignerCertificate) {
        $signature.SignerCertificate.Thumbprint
      } else {
        ''
      }
    }
  }
)
$consoleProcesses = @(
  Get-Process -ErrorAction SilentlyContinue |
    Where-Object {
      $_.MainWindowHandle -ne 0 -and
      $_.ProcessName -in @('cmd', 'conhost', 'powershell', 'pwsh', 'WindowsTerminal')
    } |
    Select-Object ProcessName, Id, MainWindowTitle
)
$payload = [ordered]@{
  uac_enabled = (Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System' -Name EnableLUA).EnableLUA -eq 1
  uac_consent_process_present = @(
    Get-Process -Name 'consent' -ErrorAction SilentlyContinue
  ).Count -gt 0
  helpers = $helpers
  visible_console_processes = $consoleProcesses
}
` });
}

export function signedHelpersMatch(boundary) {
  const helpers = boundary?.helpers || [];
  return (
    helpers.length === 2
    && helpers.every((helper) => helper.exists && helper.signature_status === "Valid")
    && helpers[0].signer_thumbprint
    && helpers.every(
      (helper) => helper.signer_thumbprint === helpers[0].signer_thumbprint,
    )
  );
}

export async function consentToSandboxSetup() {
  const qmp = new QmpClient({ port: QMP_PORT });
  try {
    await qmp.connect();
    await qmp.sendChord(["alt", "y"]);
  } finally {
    qmp.close();
  }
}

function assertion(id, passed, detail) {
  return { id, status: passed ? "passed" : "failed", detail };
}

function consoleMonitorPaths(id) {
  if (!/^[a-z0-9-]+$/.test(id)) throw new Error("invalid console monitor id");
  const root = `${CONSOLE_MONITOR_ROOT}-${id}`;
  return {
    script: `${root}.ps1`,
    observations: `${root}.jsonl`,
    stop: `${root}.stop`,
    pid: `${root}.pid`,
  };
}

export function startVisibleConsoleMonitor(id) {
  const paths = consoleMonitorPaths(id);
  const source = String.raw`
$observations = '${paths.observations}'
$stop = '${paths.stop}'
while (-not (Test-Path -LiteralPath $stop)) {
  $visible = @(
    Get-Process -ErrorAction SilentlyContinue |
      Where-Object {
        $_.MainWindowHandle -ne 0 -and
        $_.ProcessName -in @('cmd', 'conhost', 'powershell', 'pwsh', 'WindowsTerminal')
      } |
      Select-Object ProcessName, Id, MainWindowTitle
  )
  if ($visible.Count -gt 0) {
    [ordered]@{
      observed_at = [DateTime]::UtcNow.ToString('o')
      processes = $visible
    } | ConvertTo-Json -Compress -Depth 5 |
      Add-Content -LiteralPath $observations -Encoding utf8
  }
  Start-Sleep -Milliseconds 75
}
`;
  const encoded = Buffer.from(source, "utf8").toString("base64");
  return windowsGuestProbe({ probeSource: String.raw`
$scriptPath = '${paths.script}'
$observations = '${paths.observations}'
$stop = '${paths.stop}'
$pidPath = '${paths.pid}'
Remove-Item -LiteralPath $scriptPath,$observations,$stop,$pidPath -Force -ErrorAction SilentlyContinue
[IO.File]::WriteAllBytes($scriptPath, [Convert]::FromBase64String('${encoded}'))
$monitor = Start-Process -FilePath 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe' -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',$scriptPath) -WindowStyle Hidden -PassThru
Set-Content -LiteralPath $pidPath -Value $monitor.Id -Encoding ascii
$payload = [ordered]@{
  id = '${id}'
  started = $true
  process_id = $monitor.Id
}
` });
}

export function stopVisibleConsoleMonitor(id) {
  const paths = consoleMonitorPaths(id);
  return windowsGuestProbe({ probeSource: String.raw`
$scriptPath = '${paths.script}'
$observationsPath = '${paths.observations}'
$stop = '${paths.stop}'
$pidPath = '${paths.pid}'
Set-Content -LiteralPath $stop -Value 'stop' -Encoding ascii
$monitorId = if (Test-Path -LiteralPath $pidPath) {
  [int](Get-Content -LiteralPath $pidPath -Raw)
} else {
  0
}
if ($monitorId -gt 0) {
  Wait-Process -Id $monitorId -ErrorAction SilentlyContinue
  Stop-Process -Id $monitorId -Force -ErrorAction SilentlyContinue
}
$observations = @(
  if (Test-Path -LiteralPath $observationsPath) {
    Get-Content -LiteralPath $observationsPath |
      Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
      ForEach-Object { $_ | ConvertFrom-Json }
  }
)
Remove-Item -LiteralPath $scriptPath,$observationsPath,$stop,$pidPath -Force -ErrorAction SilentlyContinue
$payload = [ordered]@{
  id = '${id}'
  stopped = $true
  observations = $observations
}
` });
}

export function openIntegratedTerminalExpression() {
  return `(async () => {
    const id = "windows-release-conpty";
    await window.__TAURI_INTERNALS__.invoke("terminal_open", {
      id,
      cwd: ${JSON.stringify(QA_ROOT)},
      cols: 100,
      rows: 24
    });
    await window.__TAURI_INTERNALS__.invoke("terminal_write", {
      id,
      data: "echo CLARK_CONPTY_OK\\r\\n"
    });
    return { opened: true, id, transport: "ConPTY" };
  })()`;
}

function closeIntegratedTerminalExpression() {
  return `window.__TAURI_INTERNALS__.invoke("terminal_close", {
    id: "windows-release-conpty"
  }).then(() => ({ closed: true }))`;
}

export function packagedConsoleSmokeProbe() {
  return windowsGuestProbe({ probeSource: String.raw`
$executable = 'C:\Users\home\AppData\Local\Clark Code\clark-desktop.exe'
$receiptPath = 'C:\Users\Public\ClarkConsoleSmoke.json'
Remove-Item -LiteralPath $receiptPath -Force -ErrorAction SilentlyContinue
$smoke = Start-Process -FilePath $executable -ArgumentList @('--windows-console-smoke', $receiptPath) -WindowStyle Hidden -Wait -PassThru
if ($smoke.ExitCode -ne 0) {
  throw "packaged console smoke exited with code $($smoke.ExitCode)"
}
if (-not (Test-Path -LiteralPath $receiptPath -PathType Leaf)) {
  throw "packaged console smoke did not write its receipt"
}
$result = Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
Remove-Item -LiteralPath $receiptPath -Force
$payload = [ordered]@{
  status = $result.status
  ordinary_exit_code = $result.ordinary_exit_code
  ordinary_output_seen = $result.ordinary_output_seen
  pty_exit_code = $result.pty_exit_code
  pty_output_seen = $result.pty_output_seen
  computer_use_permissions_observed = $null -ne $result.computer_use_permissions
}
` });
}

export function packagedSandboxSmokeProbe() {
  return windowsGuestProbe({ probeSource: String.raw`
$executable = 'C:\Users\home\AppData\Local\Clark Code\clark-desktop.exe'
$receiptPath = 'C:\Users\Public\ClarkSandboxSmoke.json'
Remove-Item -LiteralPath $receiptPath -Force -ErrorAction SilentlyContinue
$smoke = Start-Process -FilePath $executable -ArgumentList @(
  '--windows-sandbox-smoke',
  $receiptPath,
  '${QA_ROOT}'
) -WindowStyle Hidden -Wait -PassThru
if ($smoke.ExitCode -ne 0) {
  throw "packaged sandbox smoke exited with code $($smoke.ExitCode)"
}
if (-not (Test-Path -LiteralPath $receiptPath -PathType Leaf)) {
  throw "packaged sandbox smoke did not write its receipt"
}
$result = Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
Remove-Item -LiteralPath $receiptPath -Force
$payload = [ordered]@{
  status = $result.status
  containment = $result.containment
  ordinary_exit_code = $result.ordinary_exit_code
  ordinary_output_seen = $result.ordinary_output_seen
  pty_exit_code = $result.pty_exit_code
  pty_output_seen = $result.pty_output_seen
  inside_write_observed = $result.inside_write_observed
  outside_exit_code = $result.outside_exit_code
  outside_write_blocked = $result.outside_write_blocked
}
` });
}

export async function runWindowsReleaseJourney({
  installReceiptPath,
  nativeReceiptPath,
  outputDir,
}) {
  prepareOutput(outputDir);
  const install = validateWindowsInstallReceipt(
    JSON.parse(readFileSync(installReceiptPath, "utf8")),
  );
  const nativeContainment = validateNativeContainmentReceipt(
    JSON.parse(readFileSync(nativeReceiptPath, "utf8")),
    install.source_revision,
  );
  let cleanup = null;
  let observation = null;
  let firstRun = null;
  let fullAccess = null;
  let boundary = null;
  let duringSetupBoundary = null;
  let clicked = null;
  let postSetup = null;
  let restart = null;
  let integratedTerminal = null;
  let terminalBoundary = null;
  let consoleMonitor = null;
  let consoleSmoke = null;
  let consoleSmokeMonitor = null;
  let sandboxSmoke = null;
  const assertions = [];
  assertions.push(...nativeContainment.assertions);
  try {
    startVisibleConsoleMonitor("packaged-child-smoke");
    consoleSmoke = packagedConsoleSmokeProbe();
    consoleSmokeMonitor = stopVisibleConsoleMonitor("packaged-child-smoke");
    assertions.push(
      assertion(
        "ordinary_pty_and_child_processes_hidden",
        consoleSmoke.status === "passed"
          && consoleSmoke.ordinary_exit_code === 0
          && consoleSmoke.ordinary_output_seen === true
          && consoleSmoke.pty_exit_code === 0
          && consoleSmoke.pty_output_seen === true
          && consoleSmoke.computer_use_permissions_observed === true
          && consoleSmokeMonitor.observations?.length === 0,
        "pipe-backed shell, direct ConPTY executor, and Computer Use helper stayed windowless",
      ),
    );
    configureWebViewDebugPolicy({ enabled: true });
    seedWindowsNativeCredentials({ retainedAuth: localWindowsQaRetainedAuth() });
    await launchWindowsClarkCode({ qmpPort: QMP_PORT });
    evaluateWindowsClarkWebView({ expression: qaLocalSettingsExpression({ cwd: QA_ROOT }) });
    evaluateWindowsClarkWebView({
      expression: "location.reload(); ({ reloading: true })",
    });
    sleep(4_000);
    firstRun = evaluateWindowsClarkWebView({
      expression: firstRunObservationExpression(),
    });
    fullAccess = evaluateWindowsClarkWebView({
      expression: fullAccessObservationExpression(),
    });
    boundary = installedBoundaryProbe();
    assertions.push(
      assertion(
        "candidate_installer_identity",
        install.release_candidate.installed_version
          === install.release_candidate.expected_version,
        install.release_candidate.installer_sha256,
      ),
      assertion("uac_enabled", boundary.uac_enabled === true, "EnableLUA must equal 1"),
      assertion(
        "inline_sandbox_setup",
        firstRun.inline_setup_visible === true
          && firstRun.enable_button_count === 1
          && firstRun.send_disabled === true,
        "first local command is blocked behind one inline setup action",
      ),
      assertion(
        "sandbox_state_outside_install_root",
        install.release_candidate.sandbox_state_outside_install_root === true,
        "sandbox enrollment is durable product data, not replaceable install data",
      ),
      assertion(
        "full_access_labeled_unsandboxed",
        fullAccess.full_access_visible === true
          && fullAccess.full_access_warning_visible === true,
        FULL_ACCESS_WARNING,
      ),
      assertion(
        "signed_sandbox_helpers",
        signedHelpersMatch(boundary),
        "runner and elevated setup helper must share one valid Authenticode identity",
      ),
    );

    const preConsentPassed = assertions.every((item) => item.status === "passed");
    if (preConsentPassed) {
      startVisibleConsoleMonitor("first-run");
      clicked = evaluateWindowsClarkWebView({
        expression: clickInlineSetupExpression(),
      });
      sleep(1_500);
      observation = await captureUtmWindowObservation({
        platform: "windows",
        vmName: VM_NAME,
        qmpPort: QMP_PORT,
        outputDir: path.join(outputDir, "uac-evidence"),
      });
      duringSetupBoundary = installedBoundaryProbe();
      await consentToSandboxSetup();
      postSetup = waitForPostSetup();
      integratedTerminal = evaluateWindowsClarkWebView({
        expression: openIntegratedTerminalExpression(),
      });
      sleep(1_000);
      terminalBoundary = installedBoundaryProbe();
      evaluateWindowsClarkWebView({
        expression: closeIntegratedTerminalExpression(),
      });
      configureWebViewDebugPolicy({ enabled: false });
      sandboxSmoke = packagedSandboxSmokeProbe();
      consoleMonitor = stopVisibleConsoleMonitor("first-run");
      assertions.push(
        assertion(
          "trusted_uac_consent_observed",
          observation.gui_visible === true
            && observation.capture_transport === "macos_window_id"
            && /^[0-9a-f]{64}$/.test(observation.screenshot_sha256 || "")
            && duringSetupBoundary.uac_consent_process_present === true,
          "fresh exact-VM capture and the Windows consent process prove the inline UAC boundary",
        ),
        assertion(
          "sandbox_enforced_after_setup",
          clicked.clicked === true
            && postSetup.sandbox?.state === "enforced"
            && postSetup.inline_setup_visible === false
            && postSetup.setup_error_visible === false,
          "one explicit consent reaches enforced state and removes the setup gate",
        ),
        assertion(
          "no_visible_console_windows",
          boundary.visible_console_processes?.length === 0
            && duringSetupBoundary.visible_console_processes?.length === 0
            && terminalBoundary.visible_console_processes?.length === 0
            && consoleMonitor.observations?.length === 0,
          "no cmd, PowerShell, conhost, or Windows Terminal top-level windows before, during setup, or while ConPTY is active",
        ),
        assertion(
          "integrated_terminal_uses_hidden_conpty",
          integratedTerminal.opened === true
            && integratedTerminal.transport === "ConPTY"
            && terminalBoundary.visible_console_processes?.length === 0,
          "the explicit terminal stays inside Clark's xterm surface",
        ),
        assertion(
          "packaged_sandbox_command_execution",
          sandboxSmoke.status === "passed"
            && sandboxSmoke.containment === "managed"
            && sandboxSmoke.ordinary_exit_code === 0
            && sandboxSmoke.ordinary_output_seen === true
            && sandboxSmoke.pty_exit_code === 0
            && sandboxSmoke.pty_output_seen === true
            && sandboxSmoke.inside_write_observed === true
            && sandboxSmoke.outside_write_blocked === true,
          "the installed app executed pipe and PTY commands through the enrolled sandbox and blocked an outside write",
        ),
      );

      configureWebViewDebugPolicy({ enabled: true });
      await launchWindowsClarkCode({ qmpPort: QMP_PORT });
      sleep(3_000);
      restart = evaluateWindowsClarkWebView({
        expression: postSetupObservationExpression(),
      });
      assertions.push(
        assertion(
          "sandbox_enforced_after_restart",
          restart.sandbox?.state === "enforced",
          "setup marker and policy attestation survive app restart",
        ),
      );
    }
  } finally {
    if (!consoleSmokeMonitor) {
      try {
        consoleSmokeMonitor = stopVisibleConsoleMonitor(
          "packaged-child-smoke",
        );
      } catch {
        // The monitor may not have started if the packaged probe failed early.
      }
    }
    if (!consoleMonitor) {
      try {
        consoleMonitor = stopVisibleConsoleMonitor("first-run");
      } catch {
        // The monitor is absent when pre-consent assertions fail.
      }
    }
    try {
      cleanup = configureWebViewDebugPolicy({ enabled: false });
    } catch (error) {
      cleanup = { policy_state_matches: false, error: String(error) };
    }
  }
  const assertionStatuses = new Map(
    assertions.map((item) => [item.id, item.status]),
  );
  const receipt = {
    schema_version: 1,
    receipt_type: "clark_code_windows_packaged_first_run",
    status: assertions.length === WINDOWS_FIRST_RUN_ASSERTIONS.length
      && WINDOWS_FIRST_RUN_ASSERTIONS.every(
        (id) => assertionStatuses.get(id) === "passed",
      )
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
    native_containment: nativeContainment,
    assertions,
    first_run: firstRun,
    full_access: fullAccess,
    installed_boundary: boundary,
    during_setup_boundary: duringSetupBoundary,
    integrated_terminal: integratedTerminal,
    terminal_boundary: terminalBoundary,
    console_monitor: consoleMonitor,
    packaged_console_smoke: consoleSmoke,
    packaged_console_smoke_monitor: consoleSmokeMonitor,
    packaged_sandbox_smoke: sandboxSmoke,
    setup_click: clicked,
    post_setup: postSetup,
    restart,
    uac_observation: observation,
    cleanup,
    paid_calls_made: false,
  };
  const receiptPath = path.join(outputDir, "receipt.json");
  writeFileSync(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o600 });
  chmodSync(receiptPath, 0o600);
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

async function main() {
  const args = process.argv.slice(2);
  const installReceipt = valueArg(args, "--install-receipt");
  const nativeReceipt = valueArg(args, "--native-receipt");
  const output = valueArg(args, "--out");
  if (!installReceipt || !nativeReceipt || !output) {
    throw new Error(
      "usage: utm-windows-release-journey.mjs --install-receipt FILE --native-receipt FILE --out NEW_DIRECTORY",
    );
  }
  const result = await runWindowsReleaseJourney({
    installReceiptPath: path.resolve(installReceipt),
    nativeReceiptPath: path.resolve(nativeReceipt),
    outputDir: path.resolve(output),
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
