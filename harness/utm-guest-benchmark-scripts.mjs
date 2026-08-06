function safeRunId(value) {
  const runId = String(value);
  if (!/^[a-z0-9-]{1,48}$/.test(runId)) {
    throw new Error("guest benchmark run id must be lowercase alphanumeric with hyphens");
  }
  return runId;
}

export function ubuntuPythonParserProbe(source) {
  const encodedSource = Buffer.from(String(source), "utf8").toString("base64");
  return `import base64
source = base64.b64decode("${encodedSource}").decode("utf-8")
try:
    compile(source, "<clark-qa-guest-benchmark>", "exec")
    payload = {"syntax_valid": True, "errors": []}
except SyntaxError as error:
    payload = {
        "syntax_valid": False,
        "errors": [{
            "message": str(error.msg),
            "line": error.lineno,
            "column": error.offset,
            "text": (error.text or "").strip(),
        }],
    }
`;
}

export function ubuntuOfflineBenchmarkProbe({ runId }) {
  const safeId = safeRunId(runId);
  return `import hashlib, json, os, pathlib, pwd, re, signal, subprocess, time

run_id = "${safeId}"
qa_root = pathlib.Path("/opt/clark-qa")
source_pointer = qa_root / "source-current.txt"
if not source_pointer.is_file():
    raise RuntimeError("staged source pointer is missing")
source_root = pathlib.Path(source_pointer.read_text().strip())
source_marker = source_root / ".source-sha256"
if not source_marker.is_file():
    raise RuntimeError("staged source marker is missing")
source_sha256 = source_marker.read_text().strip()
if not re.fullmatch(r"[a-f0-9]{64}", source_sha256):
    raise RuntimeError("staged source marker is invalid")

home_user = pwd.getpwnam("home")
run_root = qa_root / "runs" / run_id
matrix_root = run_root / "matrix"
target_root = qa_root / "cargo-target" / source_sha256[:12]
browser_root = qa_root / "playwright"
for directory in [run_root, target_root, browser_root]:
    directory.mkdir(parents=True, exist_ok=True)
    os.chown(directory, home_user.pw_uid, home_user.pw_gid)

guest_env = {
    "HOME": "/home/home",
    "USER": "home",
    "LOGNAME": "home",
    "CI": "1",
    "RUSTUP_HOME": "/home/home/.rustup",
    "CARGO_HOME": "/home/home/.cargo",
    "CARGO_TARGET_DIR": str(target_root),
    "XDG_CACHE_HOME": "/home/home/.cache",
    "PLAYWRIGHT_BROWSERS_PATH": str(browser_root),
    "PLAYWRIGHT_SKIP_BROWSER_GC": "1",
    "PATH": (
        "/home/home/.cargo/bin:"
        "/opt/clark-qa/node-v24.14.0-linux-arm64/bin:"
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
    ),
}

def redact(text):
    value = re.sub(r"\\bck_(?:live|test)_[A-Za-z0-9._-]+\\b", "ck_[REDACTED]", text)
    value = re.sub(
        r"(?i)(authorization\\s*[:=]\\s*bearer\\s+)\\S+",
        r"\\1[REDACTED]",
        value,
    )
    return value[-4000:]

def run_step(step_id, args, timeout_seconds):
    log_path = run_root / f"{step_id}.log"
    command = [
        "runuser", "-u", "home", "--", "env",
        *[f"{name}={value}" for name, value in guest_env.items()],
        *args,
    ]
    started = time.monotonic()
    timed_out = False
    with log_path.open("w", encoding="utf-8", errors="replace") as log:
        process = subprocess.Popen(
            command,
            cwd=source_root,
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        try:
            exit_code = process.wait(timeout=timeout_seconds)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(process.pid, signal.SIGKILL)
            exit_code = process.wait()
    output_tail = redact(log_path.read_text(encoding="utf-8", errors="replace"))
    return {
        "id": step_id,
        "status": "passed" if exit_code == 0 and not timed_out else "failed",
        "exit_code": exit_code,
        "timed_out": timed_out,
        "duration_ms": round((time.monotonic() - started) * 1000),
        "log_path": str(log_path),
        "output_tail": output_tail,
    }

steps = []
steps.append(run_step(
    "app_install",
    ["corepack", "pnpm@10", "--dir", "app", "install", "--frozen-lockfile"],
    1800,
))
if steps[-1]["status"] == "passed":
    steps.append(run_step(
        "harness_install",
        ["corepack", "pnpm@10", "--dir", "harness", "install", "--frozen-lockfile"],
        1800,
    ))
if all(step["status"] == "passed" for step in steps):
    steps.append(run_step(
        "playwright_install",
        [
            "corepack", "pnpm@10", "--dir", "harness", "exec",
            "playwright", "install", "--only-shell", "chromium",
        ],
        1800,
    ))
if all(step["status"] == "passed" for step in steps):
    steps.append(run_step(
        "offline_matrix",
        [
            "node", "harness/feature-matrix.mjs",
            "--offline", "--platform", "ubuntu",
            "--out", str(matrix_root),
        ],
        10800,
    ))

report_path = matrix_root / "report.json"
report = None
report_sha256 = None
if report_path.is_file():
    report_bytes = report_path.read_bytes()
    report_sha256 = hashlib.sha256(report_bytes).hexdigest()
    report = json.loads(report_bytes)
passed = (
    len(steps) == 4
    and all(step["status"] == "passed" for step in steps)
    and report is not None
    and report.get("status") == "passed"
    and report.get("platform") == "ubuntu"
    and report.get("execution", {}).get("mode") == "offline"
)
payload = {
    "platform": "ubuntu",
    "status": "passed" if passed else "failed",
    "execution_user": "home",
    "source_root": str(source_root),
    "source_sha256": source_sha256,
    "run_root": str(run_root),
    "report_path": str(report_path),
    "report_present": report_path.is_file(),
    "report_sha256": report_sha256,
    "report_status": report.get("status") if report else None,
    "report_summary": report.get("summary") if report else None,
    "steps": steps,
    "required_user_vm_actions": 0,
    "manual_vm_actions_allowed": False,
    "human_input_observed": False,
    "credential_recorded": False,
}
`;
}

export function windowsOfflineBenchmarkProbe({ runId }) {
  const safeId = safeRunId(runId);
  return String.raw`
$runId = "${safeId}"
$qaRoot = "C:\ClarkQA"
$sourcePointer = Join-Path $qaRoot "source-current.txt"
if (-not (Test-Path -LiteralPath $sourcePointer)) { throw "staged source pointer is missing" }
$sourceRoot = (Get-Content -LiteralPath $sourcePointer -Raw).Trim()
$sourceMarker = Join-Path $sourceRoot ".source-sha256"
if (-not (Test-Path -LiteralPath $sourceMarker)) { throw "staged source marker is missing" }
$sourceSha256 = (Get-Content -LiteralPath $sourceMarker -Raw).Trim()
if ($sourceSha256 -notmatch "^[a-f0-9]{64}$") { throw "staged source marker is invalid" }

$runRoot = Join-Path (Join-Path $qaRoot "runs") $runId
$matrixRoot = Join-Path $runRoot "matrix"
$targetRoot = Join-Path (Join-Path $qaRoot "cargo-target") $sourceSha256.Substring(0,12)
$browserRoot = Join-Path $qaRoot "playwright"
New-Item -ItemType Directory -Force -Path $runRoot,$targetRoot,$browserRoot | Out-Null
$nodeHome = Join-Path $qaRoot "tools\node-v24.14.0-win-x64"
$node = Join-Path $nodeHome "node.exe"
$corepack = Join-Path $nodeHome "corepack.cmd"
$gitHome = Join-Path $qaRoot "tools\mingit"
$clangHome = Join-Path $qaRoot "vs\VC\Tools\Llvm\x64\bin"
$env:RUSTUP_HOME = Join-Path $qaRoot "rustup"
$env:CARGO_HOME = Join-Path $qaRoot "cargo"
$env:CARGO_TARGET_DIR = $targetRoot
$env:CI = "1"
$env:PLAYWRIGHT_BROWSERS_PATH = $browserRoot
$env:PLAYWRIGHT_SKIP_BROWSER_GC = "1"
$env:Path = @(
  $nodeHome,
  (Join-Path $gitHome "cmd"),
  $clangHome,
  (Join-Path $env:CARGO_HOME "bin"),
  $env:Path
) -join ";"

$vsDevCmd = Join-Path $qaRoot "vs\Common7\Tools\VsDevCmd.bat"
if (-not (Test-Path -LiteralPath $vsDevCmd)) { throw "Visual Studio developer environment is missing" }
$rustcHost = @(& (Join-Path $env:CARGO_HOME "bin\rustc.exe") -vV) |
  Where-Object { $_ -like "host:*" } |
  Select-Object -First 1
$vsArch = if ($rustcHost -eq "host: aarch64-pc-windows-msvc") { "arm64" } else { "x64" }
$vsEnvironmentCommand = '"' + $vsDevCmd + '" -arch=' + $vsArch + ' -host_arch=x64 >nul && set'
$environmentLines = & cmd.exe /d /s /c $vsEnvironmentCommand
foreach ($line in $environmentLines) {
  $separator = $line.IndexOf("=")
  if ($separator -gt 0) {
    [Environment]::SetEnvironmentVariable(
      $line.Substring(0, $separator),
      $line.Substring($separator + 1),
      "Process"
    )
  }
}

function Redact-Diagnostic([string]$Text) {
  $safe = $Text -replace "\bck_(?:live|test)_[A-Za-z0-9._-]+\b", "ck_[REDACTED]"
  $safe = $safe -replace "(?i)(authorization\s*[:=]\s*bearer\s+)\S+", '$1[REDACTED]'
  if ($safe.Length -gt 4000) { return $safe.Substring($safe.Length - 4000) }
  return $safe
}

function Invoke-Step(
  [string]$Id,
  [string]$FilePath,
  [string[]]$Arguments,
  [int]$TimeoutSeconds
) {
  $stdoutPath = Join-Path $runRoot "$Id.stdout.log"
  $stderrPath = Join-Path $runRoot "$Id.stderr.log"
  $started = Get-Date
  $timedOut = $false
  $process = Start-Process -FilePath $FilePath -ArgumentList $Arguments -WorkingDirectory $sourceRoot -PassThru -NoNewWindow -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
  if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
    $timedOut = $true
    & taskkill.exe /PID $process.Id /T /F | Out-Null
  }
  $process.WaitForExit()
  $process.Refresh()
  $exitCode = [int]$process.ExitCode
  $output = @(
    Get-Content -LiteralPath $stdoutPath -Raw -ErrorAction SilentlyContinue
    Get-Content -LiteralPath $stderrPath -Raw -ErrorAction SilentlyContinue
  ) -join [Environment]::NewLine
  return [ordered]@{
    id = $Id
    status = if ($exitCode -eq 0 -and -not $timedOut) { "passed" } else { "failed" }
    exit_code = $exitCode
    timed_out = $timedOut
    duration_ms = [math]::Round(((Get-Date) - $started).TotalMilliseconds)
    stdout_path = $stdoutPath
    stderr_path = $stderrPath
    output_tail = Redact-Diagnostic $output
  }
}

$steps = [Collections.Generic.List[object]]::new()
$steps.Add((Invoke-Step "app_install" $corepack @(
  "pnpm@10", "--dir", "app", "install", "--frozen-lockfile"
) 1800))
if ($steps[$steps.Count - 1].status -eq "passed") {
  $steps.Add((Invoke-Step "harness_install" $corepack @(
    "pnpm@10", "--dir", "harness", "install", "--frozen-lockfile"
  ) 1800))
}
if (@($steps | Where-Object { $_.status -ne "passed" }).Count -eq 0) {
  $steps.Add((Invoke-Step "playwright_install" $corepack @(
    "pnpm@10", "--dir", "harness", "exec",
    "playwright", "install", "--only-shell", "chromium"
  ) 1800))
}
if (@($steps | Where-Object { $_.status -ne "passed" }).Count -eq 0) {
  $steps.Add((Invoke-Step "offline_matrix" $node @(
    "harness/feature-matrix.mjs",
    "--offline", "--platform", "windows",
    "--out", $matrixRoot
  ) 10800))
}

$reportPath = Join-Path $matrixRoot "report.json"
$report = $null
$reportSha256 = $null
if (Test-Path -LiteralPath $reportPath) {
  $reportSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $reportPath).Hash.ToLower()
  $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
}
$passed = ($steps.Count -eq 4 -and @($steps | Where-Object { $_.status -ne "passed" }).Count -eq 0 -and $null -ne $report -and $report.status -eq "passed" -and $report.platform -eq "windows" -and $report.execution.mode -eq "offline")
$payload = [ordered]@{
  platform = "windows"
  status = if ($passed) { "passed" } else { "failed" }
  execution_user = [Security.Principal.WindowsIdentity]::GetCurrent().Name
  source_root = $sourceRoot
  source_sha256 = $sourceSha256
  run_root = $runRoot
  report_path = $reportPath
  report_present = Test-Path -LiteralPath $reportPath
  report_sha256 = $reportSha256
  report_status = if ($report) { $report.status } else { $null }
  report_summary = if ($report) { $report.summary } else { $null }
  steps = @($steps)
  required_user_vm_actions = 0
  manual_vm_actions_allowed = $false
  human_input_observed = $false
  credential_recorded = $false
}
`;
}
