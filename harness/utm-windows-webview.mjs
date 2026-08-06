#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { Buffer } from "node:buffer";
import process from "node:process";
import { pathToFileURL } from "node:url";

import { assertClarkOwnedQaEmail } from "./clark-qa-auth.mjs";
import { nativeCredentialEnvelope } from "./native-credential-envelope.mjs";
import { executeGuestJson } from "./utm-guest-channel.mjs";
import { QmpClient } from "./utm-qmp.mjs";

const DEFAULT_VM_NAME =
  process.env.CLARK_WINDOWS_QA_VM_NAME || "Clark QA - Windows 11 ARM";
const DEFAULT_QMP_PORT = 47_111;
const DEFAULT_CDP_PORT = 9_222;
const DEFAULT_EXECUTABLE =
  String.raw`C:\Users\home\AppData\Local\Clark Code\clark-desktop.exe`;
const POLICY_NAMES = ["clark-desktop.exe", "com.clark.desktop", "*"];

function redact(value) {
  return String(value)
    .replace(/\bck_(?:live|test)_[A-Za-z0-9._-]+\b/g, "ck_[REDACTED]")
    .replace(/\bsk-[A-Za-z0-9_-]{16,}\b/g, "sk-[REDACTED]")
    .replace(/\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b/g, "[JWT_REDACTED]")
    .replace(/(authorization\s*[:=]\s*bearer\s+)\S+/gi, "$1[REDACTED]")
    .slice(-8_000);
}

function defaultRun(command, args, options = {}) {
  const completed = spawnSync(command, args, {
    encoding: "utf8",
    input: options.input,
    timeout: options.timeout_ms ?? 30_000,
    maxBuffer: 8 * 1024 * 1024,
  });
  return {
    ok: completed.status === 0,
    exit_code: completed.status,
    stdout: redact(completed.stdout || ""),
    stderr: redact(completed.stderr || completed.error?.message || ""),
  };
}

function powershellString(value) {
  return `'${String(value).replaceAll("'", "''")}'`;
}

export function windowsGuestProbe({
  probeSource,
  vmName = DEFAULT_VM_NAME,
  run = defaultRun,
  timeoutMs = 45_000,
}) {
  const probe = executeGuestJson({
    platform: "windows",
    vmName,
    state: "started",
    probeSource,
    run,
    timeoutMs,
  });
  if (!probe.ok) {
    throw new Error(`Windows guest probe failed: ${redact(probe.error)}`);
  }
  return probe.data;
}

export function localWindowsQaRetainedAuth() {
  const subject = "clark-code-autonomous-vm-qa";
  const payload = Buffer.from(JSON.stringify({ sub: subject })).toString("base64url");
  return {
    version: 2,
    descriptor: {
      user: { id: subject, name: "Autonomous VM QA", method: "local" },
    },
    authOrigin: "https://www.clarkchat.com",
    clarkToken: `eyJhbGciOiJub25lIn0.${payload}.signature`,
    google: { accessToken: "", refreshToken: null, expiresAt: null },
  };
}

export function seedWindowsNativeCredentials({
  retainedAuth,
  bundleIdentifier = "com.clark.desktop",
  vmName = DEFAULT_VM_NAME,
  run = defaultRun,
}) {
  if (!/^com\.clark\.desktop(?:\.dev)?$/.test(bundleIdentifier)) {
    throw new Error("Windows credential bootstrap requires a Clark Desktop bundle identifier");
  }
  const files = nativeCredentialEnvelope(retainedAuth);
  return windowsGuestProbe({
    vmName,
    run,
    probeSource: String.raw`
Get-Process clark-desktop -ErrorAction SilentlyContinue | Stop-Process -Force
$root = Join-Path $env:APPDATA ${powershellString(`${bundleIdentifier}\\credentials`)}
New-Item -ItemType Directory -Force -Path $root | Out-Null
[IO.File]::WriteAllBytes(
  (Join-Path $root "credentials.key"),
  [Convert]::FromBase64String(${powershellString(files.key)})
)
[IO.File]::WriteAllBytes(
  (Join-Path $root "credentials.enc"),
  [Convert]::FromBase64String(${powershellString(files.envelope)})
)
icacls $root /inheritance:r /grant:r "$($env:USERNAME):(OI)(CI)F" | Out-Null
$payload = [ordered]@{
  status = if (
    (Test-Path (Join-Path $root "credentials.key")) -and
    (Test-Path (Join-Path $root "credentials.enc"))
  ) { "passed" } else { "failed" }
  credential_storage = "app_owned_encrypted_disk"
  credential_recorded = $false
}
`,
  });
}

export function buildWebViewPolicyProbe({ enabled, cdpPort = DEFAULT_CDP_PORT }) {
  if (!Number.isInteger(cdpPort) || cdpPort < 1 || cdpPort > 65_535) {
    throw new Error("CDP port must be an integer from 1 through 65535");
  }
  const names = POLICY_NAMES.map(powershellString).join(",");
  const desired =
    `--remote-debugging-port=${cdpPort} `
    + `--remote-allow-origins=http://127.0.0.1:${cdpPort}`;
  const mutation = enabled
    ? `
$policyKey = $root.CreateSubKey($registryPath)
foreach ($name in $names) {
  $policyKey.SetValue($name, $desired, [Microsoft.Win32.RegistryValueKind]::String)
}`
    : `
$policyKey = $root.OpenSubKey($registryPath, $true)
if ($policyKey) {
  foreach ($name in $names) {
    $policyKey.DeleteValue($name, $false)
  }
}`;
  return String.raw`
$registryPath = "Software\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments"
$root = [Microsoft.Win32.Registry]::LocalMachine
$names = @(${names})
$desired = ${powershellString(desired)}
Get-Process clark-desktop -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2
${mutation}
$matching = 0
if ($policyKey) {
  foreach ($name in $names) {
    $value = $policyKey.GetValue($name, $null)
    if ($value -eq $desired) { $matching += 1 }
  }
  $policyKey.Close()
}
$payload = [ordered]@{
  requested_enabled = ${enabled ? "$true" : "$false"}
  matching_policy_values = $matching
  policy_state_matches = ${enabled ? "$matching -eq $names.Count" : "$matching -eq 0"}
  app_stopped = @(Get-Process clark-desktop -ErrorAction SilentlyContinue).Count -eq 0
  cdp_bind = "guest_loopback"
  cdp_port = ${cdpPort}
}
`;
}

export function configureWebViewDebugPolicy({
  enabled,
  vmName = DEFAULT_VM_NAME,
  cdpPort = DEFAULT_CDP_PORT,
  run = defaultRun,
}) {
  const data = windowsGuestProbe({
    vmName,
    run,
    probeSource: buildWebViewPolicyProbe({ enabled, cdpPort }),
  });
  if (!data.policy_state_matches || !data.app_stopped) {
    throw new Error(
      `WebView debug policy did not reach requested state: ${JSON.stringify(data)}`,
    );
  }
  return data;
}

export async function launchWindowsClarkCode({
  executable = DEFAULT_EXECUTABLE,
  qmpPort = DEFAULT_QMP_PORT,
  settleMs = 4_000,
}) {
  if (!/^[A-Za-z]:\\/.test(executable)) {
    throw new Error("Clark Code executable must be an absolute Windows path");
  }
  const qmp = new QmpClient({ port: qmpPort });
  try {
    await qmp.connect();
    await qmp.openWindowsRunAndExecute(executable, { settleMs: 900 });
  } finally {
    qmp.close();
  }
  await new Promise((resolve) => setTimeout(resolve, settleMs));
  return {
    launch_transport: "localhost_qmp_keyboard",
    executable,
    required_user_vm_actions: 0,
  };
}

export function buildCdpEvaluationProbe({
  expression,
  cdpPort = DEFAULT_CDP_PORT,
}) {
  const encodedExpression = Buffer.from(String(expression), "utf8").toString("base64");
  return String.raw`
$expression = [Text.Encoding]::UTF8.GetString(
  [Convert]::FromBase64String(${powershellString(encodedExpression)})
)
$rawTargets = (
  Invoke-WebRequest -UseBasicParsing "http://127.0.0.1:${cdpPort}/json/list" -TimeoutSec 5
).Content
$targets = $rawTargets | ConvertFrom-Json
$target = $targets |
  Where-Object { $_.type -eq "page" -and $_.url -like "http://tauri.localhost*" } |
  Select-Object -First 1
if (-not $target) { throw "Clark Code CDP target not found" }
$socket = [System.Net.WebSockets.ClientWebSocket]::new()
$socket.Options.SetRequestHeader("Origin", "http://127.0.0.1:${cdpPort}")
$socket.ConnectAsync(
  [Uri]$target.webSocketDebuggerUrl,
  [Threading.CancellationToken]::None
).GetAwaiter().GetResult()
$request = @{
  id = 1
  method = "Runtime.evaluate"
  params = @{
    expression = $expression
    returnByValue = $true
    awaitPromise = $true
  }
} | ConvertTo-Json -Compress -Depth 12
$bytes = [Text.Encoding]::UTF8.GetBytes($request)
$socket.SendAsync(
  [ArraySegment[byte]]::new($bytes),
  [Net.WebSockets.WebSocketMessageType]::Text,
  $true,
  [Threading.CancellationToken]::None
).GetAwaiter().GetResult()
$response = $null
while (-not $response) {
  $stream = [IO.MemoryStream]::new()
  do {
    $buffer = New-Object byte[] 65536
    $received = $socket.ReceiveAsync(
      [ArraySegment[byte]]::new($buffer),
      [Threading.CancellationToken]::None
    ).GetAwaiter().GetResult()
    $stream.Write($buffer, 0, $received.Count)
  } while (-not $received.EndOfMessage)
  $candidate = (
    [Text.Encoding]::UTF8.GetString($stream.ToArray()) | ConvertFrom-Json
  )
  if ($candidate.id -eq 1) { $response = $candidate }
}
$socket.Dispose()
if ($response.error) { throw "CDP returned a protocol error" }
if ($response.result.exceptionDetails) { throw "Clark Code JavaScript evaluation failed" }
$payload = [ordered]@{
  cdp_connected = $true
  cdp_bind = "guest_loopback"
  result_type = $response.result.result.type
  value = $response.result.result.value
}
`;
}

export function evaluateWindowsClarkWebView({
  expression,
  vmName = DEFAULT_VM_NAME,
  cdpPort = DEFAULT_CDP_PORT,
  run = defaultRun,
}) {
  const data = windowsGuestProbe({
    vmName,
    run,
    probeSource: buildCdpEvaluationProbe({ expression, cdpPort }),
  });
  if (!data.cdp_connected) throw new Error("Clark Code CDP evaluation did not connect");
  return data.value;
}

export function qaLocalSettingsExpression({
  cwd = String.raw`C:\Users\home\ClarkCodeQA`,
  model = "clark-code:deepseek_v4_flash_latest",
}) {
  const localSettings = {
    cwd: "",
    model,
    reasoningEffort: "",
    computerUseEnabled: false,
  };
  return `(() => {
    const owner = "id:clark-code-autonomous-vm-qa";
    const encodedOwner = encodeURIComponent(owner);
    localStorage.clear();
    localStorage.setItem(
      "clark-desktop:local-agent:" + encodedOwner,
      ${JSON.stringify(JSON.stringify(localSettings))}
    );
    localStorage.setItem(
      "clark-desktop:project-context:" + encodedOwner,
      ${JSON.stringify(JSON.stringify({ cwd }))}
    );
    return {
      auth_fixture: "native_encrypted_local_account",
      project_configured: true,
      model: ${JSON.stringify(model)},
      credential_fields_absent: !/apiKey|apiKeyOwner|clarkToken|refreshToken|accessToken/i.test(
        JSON.stringify(Object.fromEntries(Object.entries(localStorage)))
      )
    };
  })()`;
}

export function qaAccountSettingsExpression({
  account,
  cwd = String.raw`C:\Users\home\ClarkCodeQA`,
  model = "clark-code:deepseek_v4_flash_latest",
}) {
  const id = account?.id?.trim();
  const name = account?.name?.trim();
  const email = account?.email?.trim();
  if (!id || !name || !email) {
    throw new Error("authenticated Windows QA settings require a complete account descriptor");
  }
  assertClarkOwnedQaEmail(email);
  const localSettings = {
    cwd: "",
    model,
    reasoningEffort: "",
    computerUseEnabled: false,
  };
  return `(() => {
    const owner = ${JSON.stringify(`id:${id}`)};
    const encodedOwner = encodeURIComponent(owner);
    const settingsKey = "clark-desktop:local-agent:" + encodedOwner;
    localStorage.clear();
    localStorage.setItem(
      settingsKey,
      ${JSON.stringify(JSON.stringify(localSettings))}
    );
    localStorage.setItem(
      "clark-desktop:project-context:" + encodedOwner,
      ${JSON.stringify(JSON.stringify({ cwd }))}
    );
    return {
      auth_fixture: "native_encrypted_account",
      account_bound: true,
      project_configured: true,
      model: ${JSON.stringify(model)},
      credential_fields_absent: !/apiKey|apiKeyOwner|clarkToken|refreshToken|accessToken/i.test(
        JSON.stringify(Object.fromEntries(Object.entries(localStorage)))
      )
    };
  })()`;
}

async function runCli() {
  const [command = "smoke", ...args] = process.argv.slice(2);
  if (command === "--help" || command === "-h" || args.length > 0) {
    console.log(`Autonomous Windows Clark Code WebView control

Usage:
  node harness/utm-windows-webview.mjs smoke

The smoke command enables a guest-local, temporary WebView2 debugging policy,
launches the installed Clark Code release through localhost-only UTM QMP,
proves DOM access through guest-local CDP, then removes the policy and stops
the app. It never requires user input and never records a credential.`);
    if (command === "--help" || command === "-h") return;
    throw new Error(`unknown arguments ${JSON.stringify([command, ...args])}`);
  }
  if (command !== "smoke") throw new Error(`unknown command ${JSON.stringify(command)}`);

  let cleanup = null;
  try {
    const enabled = configureWebViewDebugPolicy({ enabled: true });
    await launchWindowsClarkCode({});
    const observed = evaluateWindowsClarkWebView({
      expression:
        `({title: document.title, url: location.href, `
        + `signInVisible: document.body.innerText.includes("Continue with Google")})`,
    });
    if (
      observed?.title !== "Clark Code"
      || observed?.url !== "http://tauri.localhost/"
      || observed?.signInVisible !== true
    ) {
      throw new Error(`unexpected Clark Code DOM: ${JSON.stringify(observed)}`);
    }
    cleanup = configureWebViewDebugPolicy({ enabled: false });
    console.log(JSON.stringify({
      status: "passed",
      policy_enabled: enabled.policy_state_matches,
      dom: observed,
      cleanup: cleanup.policy_state_matches,
      required_user_vm_actions: 0,
    }));
  } finally {
    if (!cleanup) {
      try {
        configureWebViewDebugPolicy({ enabled: false });
      } catch {
        // Preserve the original failure; the caller can rerun the idempotent
        // cleanup command by invoking this smoke again.
      }
    }
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await runCli();
}
