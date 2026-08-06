export const NODE_VERSION = "24.14.0";

export function windowsPowerShellParserProbe(source) {
  const encodedSource = Buffer.from(String(source), "utf8").toString("base64");
  return [
    `$source = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String("${encodedSource}"))`,
    "$tokens = $null",
    "$errors = $null",
    "[System.Management.Automation.Language.Parser]::ParseInput($source, [ref]$tokens, [ref]$errors) | Out-Null",
    "$payload = [ordered]@{",
    "  syntax_valid = @($errors).Count -eq 0",
    "  errors = @($errors | ForEach-Object {",
    "    [ordered]@{",
    "      message = $_.Message",
    "      line = $_.Extent.StartLineNumber",
    "      column = $_.Extent.StartColumnNumber",
    "      text = $_.Extent.Text",
    "    }",
    "  })",
    "}",
  ].join("\n");
}

export function ubuntuProvisionProbe() {
  return `import hashlib, os, pathlib, pwd, shutil, subprocess, urllib.request

node_version = "${NODE_VERSION}"
qa_root = pathlib.Path("/opt/clark-qa")
qa_root.mkdir(parents=True, exist_ok=True)
source_pointer = qa_root / "source-current.txt"
if not source_pointer.is_file():
    raise RuntimeError("staged source pointer is missing")
source_root = pathlib.Path(source_pointer.read_text().strip())
source_marker = source_root / ".source-sha256"
if not source_marker.is_file():
    raise RuntimeError("staged source SHA-256 marker is missing")
source_sha256 = source_marker.read_text().strip()
if os.geteuid() != 0:
    raise RuntimeError("Ubuntu provisioning must run through the root guest agent")

def run(args, *, env=None, timeout=3600):
    completed = subprocess.run(
        args,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=timeout,
    )
    if completed.returncode != 0:
        tail = completed.stdout[-2000:].replace("\\n", " ")
        raise RuntimeError(f"{args[0]} exited {completed.returncode}: {tail}")
    return completed.stdout.strip()

apt_env = dict(os.environ)
apt_env.update({
    "DEBIAN_FRONTEND": "noninteractive",
    "NEEDRESTART_MODE": "a",
})
packages = [
    "apparmor",
    "build-essential",
    "bubblewrap",
    "ca-certificates",
    "clang",
    "cmake",
    "curl",
    "file",
    "git",
    "libayatana-appindicator3-dev",
    "libdbus-1-dev",
    "libgtk-3-dev",
    "libjavascriptcoregtk-4.1-dev",
    "libsecret-1-dev",
    "libssl-dev",
    "libwebkit2gtk-4.1-dev",
    "libxdo-dev",
    "libxkbcommon-dev",
    "lld",
    "ninja-build",
    "patchelf",
    "pkg-config",
    "protobuf-compiler",
    "python3",
    "xz-utils",
]
run(["apt-get", "update"], env=apt_env, timeout=1200)
run(["apt-get", "install", "-y", *packages], env=apt_env, timeout=3600)

def bwrap_probe():
    return subprocess.run(
        [
            "runuser", "-u", "home", "--", "/usr/bin/bwrap",
            "--ro-bind", "/", "/", "--unshare-user", "--", "/bin/true",
        ],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=30,
    )

# Ubuntu 24.04 keeps its global AppArmor user-namespace restriction enabled.
# Give only bubblewrap the narrow userns allowance Ubuntu recommends for
# sandboxing applications; never turn the host-wide restriction off.
apparmor_restriction_path = pathlib.Path(
    "/proc/sys/kernel/apparmor_restrict_unprivileged_userns"
)
apparmor_restriction = (
    apparmor_restriction_path.read_text().strip()
    if apparmor_restriction_path.is_file()
    else None
)
apparmor_profile = pathlib.Path("/etc/apparmor.d/usr.bin.bwrap-clark-qa")
initial_bwrap = bwrap_probe()
if initial_bwrap.returncode != 0:
    apparmor_profile.write_text(
        """abi <abi/4.0>,
include <tunables/global>

/usr/bin/bwrap flags=(default_allow) {
  userns,
}
""",
        encoding="utf-8",
    )
    run(["apparmor_parser", "-r", str(apparmor_profile)], timeout=120)
final_bwrap = bwrap_probe()
if final_bwrap.returncode != 0:
    tail = final_bwrap.stdout[-1200:].replace("\\n", " ")
    raise RuntimeError(f"bubblewrap sandbox probe failed after AppArmor setup: {tail}")

node_archive = f"node-v{node_version}-linux-arm64.tar.xz"
node_url = f"https://nodejs.org/dist/v{node_version}/{node_archive}"
sums_url = f"https://nodejs.org/dist/v{node_version}/SHASUMS256.txt"
downloads = qa_root / "downloads"
downloads.mkdir(exist_ok=True)
archive_path = downloads / node_archive
sums_path = downloads / f"node-v{node_version}-SHASUMS256.txt"
if not archive_path.is_file():
    urllib.request.urlretrieve(node_url, archive_path)
if not sums_path.is_file():
    urllib.request.urlretrieve(sums_url, sums_path)
expected_node_hash = None
for line in sums_path.read_text().splitlines():
    fields = line.split()
    if len(fields) == 2 and fields[1] == node_archive:
        expected_node_hash = fields[0]
        break
if not expected_node_hash:
    raise RuntimeError("Node release checksum is absent")
actual_node_hash = hashlib.sha256(archive_path.read_bytes()).hexdigest()
if actual_node_hash != expected_node_hash:
    raise RuntimeError("Node archive SHA-256 mismatch")
node_home = qa_root / f"node-v{node_version}-linux-arm64"
if not node_home.is_dir():
    run(["tar", "-xJf", str(archive_path), "-C", str(qa_root)])
for name in ["node", "npm", "npx", "corepack"]:
    source = node_home / "bin" / name
    target = pathlib.Path("/usr/local/bin") / name
    if not source.exists():
        raise RuntimeError(f"Node distribution is missing {name}")
    if target.is_symlink() or not target.exists():
        target.unlink(missing_ok=True)
        target.symlink_to(source)
run([str(node_home / "bin" / "corepack"), "enable"])
run([str(node_home / "bin" / "corepack"), "prepare", "pnpm@10", "--activate"])

home = pwd.getpwnam("home")
rustup_home = pathlib.Path("/home/home/.rustup")
cargo_home = pathlib.Path("/home/home/.cargo")
rustup_init = downloads / "rustup-init-aarch64-unknown-linux-gnu"
rustup_url = (
    "https://static.rust-lang.org/rustup/dist/"
    "aarch64-unknown-linux-gnu/rustup-init"
)
rustup_hash_path = downloads / "rustup-init-aarch64-unknown-linux-gnu.sha256"
if not rustup_init.is_file():
    urllib.request.urlretrieve(rustup_url, rustup_init)
if not rustup_hash_path.is_file():
    urllib.request.urlretrieve(rustup_url + ".sha256", rustup_hash_path)
expected_rustup_hash = rustup_hash_path.read_text().split()[0]
actual_rustup_hash = hashlib.sha256(rustup_init.read_bytes()).hexdigest()
if actual_rustup_hash != expected_rustup_hash:
    raise RuntimeError("rustup-init SHA-256 mismatch")
rustup_init.chmod(0o755)
for item in [rustup_init, rustup_home, cargo_home]:
    if item.exists():
        os.chown(item, home.pw_uid, home.pw_gid)
if not (cargo_home / "bin" / "cargo").is_file():
    rust_env = dict(os.environ)
    rust_env.update({
        "HOME": "/home/home",
        "USER": "home",
        "RUSTUP_HOME": str(rustup_home),
        "CARGO_HOME": str(cargo_home),
    })
    run([
        "runuser", "-u", "home", "--", "env",
        f"HOME={rust_env['HOME']}",
        f"RUSTUP_HOME={rust_env['RUSTUP_HOME']}",
        f"CARGO_HOME={rust_env['CARGO_HOME']}",
        str(rustup_init),
        "-y",
        "--profile", "minimal",
        "--default-toolchain", "stable",
        "--no-modify-path",
    ], timeout=3600)
if source_root.is_dir():
    run(["chown", "-R", "home:home", str(source_root)])

tool_env = dict(os.environ)
tool_env["PATH"] = f"{cargo_home}/bin:{node_home}/bin:" + tool_env["PATH"]
tool_env["RUSTUP_HOME"] = str(rustup_home)
tool_env["CARGO_HOME"] = str(cargo_home)
payload = {
    "platform": "ubuntu",
    "architecture": run(["uname", "-m"]),
    "node_version": run([str(node_home / "bin" / "node"), "--version"]),
    "node_archive_sha256": actual_node_hash,
    "pnpm_version": run([str(node_home / "bin" / "corepack"), "pnpm@10", "--version"]),
    "rustc_version": run([str(cargo_home / "bin" / "rustc"), "--version"], env=tool_env),
    "cargo_version": run([str(cargo_home / "bin" / "cargo"), "--version"], env=tool_env),
    "rustup_init_sha256": actual_rustup_hash,
    "webkit_pkg_version": run(["pkg-config", "--modversion", "webkit2gtk-4.1"]),
    "bubblewrap_path": shutil.which("bwrap"),
    "bubblewrap_sandbox_ready": final_bwrap.returncode == 0,
    "apparmor_userns_restriction": apparmor_restriction,
    "apparmor_profile_path": (
        str(apparmor_profile) if apparmor_profile.is_file() else None
    ),
    "apparmor_profile_sha256": (
        hashlib.sha256(apparmor_profile.read_bytes()).hexdigest()
        if apparmor_profile.is_file()
        else None
    ),
    "source_root": str(source_root),
    "source_sha256": source_sha256,
    "source_present": (source_root / "Cargo.toml").is_file(),
    "reboot_required": pathlib.Path("/var/run/reboot-required").exists(),
}
`;
}

export function windowsProvisionProbe() {
  return String.raw`
$qaRoot = "C:\ClarkQA"
$toolsRoot = Join-Path $qaRoot "tools"
$downloads = Join-Path $qaRoot "downloads"
$nodeVersion = "${NODE_VERSION}"
New-Item -ItemType Directory -Force -Path $toolsRoot,$downloads | Out-Null
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$sourcePointer = Join-Path $qaRoot "source-current.txt"
if (-not (Test-Path -LiteralPath $sourcePointer)) { throw "staged source pointer is missing" }
$sourceRoot = (Get-Content -LiteralPath $sourcePointer -Raw).Trim()
$sourceMarker = Join-Path $sourceRoot ".source-sha256"
if (-not (Test-Path -LiteralPath $sourceMarker)) { throw "staged source SHA-256 marker is missing" }
$sourceSha256 = (Get-Content -LiteralPath $sourceMarker -Raw).Trim()

function Invoke-Checked([string]$FilePath, [string[]]$Arguments) {
  $captureId = [guid]::NewGuid().ToString("N")
  $stdoutPath = Join-Path $env:TEMP "clark-qa-$captureId.stdout"
  $stderrPath = Join-Path $env:TEMP "clark-qa-$captureId.stderr"
  try {
    $process = Start-Process -FilePath $FilePath -ArgumentList $Arguments -Wait -PassThru -NoNewWindow -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    if ($process.ExitCode -ne 0) {
      $diagnostic = @(
        Get-Content -LiteralPath $stdoutPath -ErrorAction SilentlyContinue
        Get-Content -LiteralPath $stderrPath -ErrorAction SilentlyContinue
      ) -join " "
      if ($diagnostic.Length -gt 1200) {
        $diagnostic = $diagnostic.Substring($diagnostic.Length - 1200)
      }
      throw "$FilePath exited $($process.ExitCode): $diagnostic"
    }
  } finally {
    Remove-Item -LiteralPath $stdoutPath,$stderrPath -Force -ErrorAction SilentlyContinue
  }
}

$nodeArchive = "node-v$nodeVersion-win-x64.zip"
$nodeUrl = "https://nodejs.org/dist/v$nodeVersion/$nodeArchive"
$sumsUrl = "https://nodejs.org/dist/v$nodeVersion/SHASUMS256.txt"
$nodeArchivePath = Join-Path $downloads $nodeArchive
$sumsPath = Join-Path $downloads "node-v$nodeVersion-SHASUMS256.txt"
if (-not (Test-Path -LiteralPath $nodeArchivePath)) {
  Invoke-WebRequest -UseBasicParsing -Uri $nodeUrl -OutFile $nodeArchivePath
}
if (-not (Test-Path -LiteralPath $sumsPath)) {
  Invoke-WebRequest -UseBasicParsing -Uri $sumsUrl -OutFile $sumsPath
}
$escapedArchive = [regex]::Escape($nodeArchive)
$sumLine = Get-Content -LiteralPath $sumsPath |
  Where-Object { $_ -match "^[a-f0-9]{64}\s+$escapedArchive$" } |
  Select-Object -First 1
if (-not $sumLine) { throw "Node release checksum is absent" }
$expectedNodeHash = ($sumLine -split "\s+")[0]
$actualNodeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $nodeArchivePath).Hash.ToLower()
if ($actualNodeHash -ne $expectedNodeHash) { throw "Node archive SHA-256 mismatch" }
$nodeHome = Join-Path $toolsRoot "node-v$nodeVersion-win-x64"
if (-not (Test-Path -LiteralPath $nodeHome)) {
  Expand-Archive -LiteralPath $nodeArchivePath -DestinationPath $toolsRoot
}
$node = Join-Path $nodeHome "node.exe"
$corepack = Join-Path $nodeHome "corepack.cmd"
if (-not (Test-Path -LiteralPath $corepack)) {
  Invoke-Checked (Join-Path $nodeHome "npm.cmd") @("install","--global","corepack@0.34.0")
}
$env:Path = "$nodeHome;$env:Path"
Invoke-Checked $corepack @("enable")
Invoke-Checked $corepack @("prepare","pnpm@10","--activate")

$gitHome = Join-Path $toolsRoot "mingit"
$headers = @{ "User-Agent" = "Clark-Code-VM-QA" }
$release = Invoke-RestMethod -Headers $headers -Uri "https://api.github.com/repos/git-for-windows/git/releases/latest"
$asset = @($release.assets | Where-Object {
  $_.name -match "^MinGit-[0-9].*-64-bit\.zip$"
}) | Select-Object -First 1
if (-not $asset -or -not $asset.digest -or $asset.digest -notmatch "^sha256:") {
  throw "Git for Windows release lacks a SHA-256 asset digest"
}
$gitArchivePath = Join-Path $downloads $asset.name
if (-not (Test-Path -LiteralPath $gitArchivePath)) {
  Invoke-WebRequest -UseBasicParsing -Headers $headers -Uri $asset.browser_download_url -OutFile $gitArchivePath
}
$expectedGitHash = $asset.digest.Substring(7).ToLower()
$actualGitHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $gitArchivePath).Hash.ToLower()
if ($actualGitHash -ne $expectedGitHash) { throw "MinGit archive SHA-256 mismatch" }
if (-not (Test-Path -LiteralPath (Join-Path $gitHome "cmd\git.exe"))) {
  New-Item -ItemType Directory -Force -Path $gitHome | Out-Null
  Expand-Archive -LiteralPath $gitArchivePath -DestinationPath $gitHome
}

$rustupUrl = "https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe"
$rustupPath = Join-Path $downloads "rustup-init-x86_64-pc-windows-msvc.exe"
$rustupHashPath = $rustupPath + ".sha256"
if (-not (Test-Path -LiteralPath $rustupPath)) {
  Invoke-WebRequest -UseBasicParsing -Uri $rustupUrl -OutFile $rustupPath
}
if (-not (Test-Path -LiteralPath $rustupHashPath)) {
  Invoke-WebRequest -UseBasicParsing -Uri ($rustupUrl + ".sha256") -OutFile $rustupHashPath
}
$expectedRustupHash = ((Get-Content -LiteralPath $rustupHashPath -Raw).Trim() -split "\s+")[0]
$actualRustupHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $rustupPath).Hash.ToLower()
if ($actualRustupHash -ne $expectedRustupHash) { throw "rustup-init SHA-256 mismatch" }
$env:RUSTUP_HOME = Join-Path $qaRoot "rustup"
$env:CARGO_HOME = Join-Path $qaRoot "cargo"
if (-not (Test-Path -LiteralPath (Join-Path $env:CARGO_HOME "bin\cargo.exe"))) {
  Invoke-Checked $rustupPath @("-y","--profile","minimal","--default-toolchain","stable","--no-modify-path")
}

$vsRoot = Join-Path $qaRoot "vs"
$vsDevCmd = Join-Path $vsRoot "Common7\Tools\VsDevCmd.bat"
$msvcRoot = Get-ChildItem -LiteralPath (Join-Path $vsRoot "VC\Tools\MSVC") -Directory -ErrorAction SilentlyContinue |
  Sort-Object Name -Descending |
  Select-Object -First 1
$arm64ToolsPresent = ($null -ne $msvcRoot -and (Test-Path -LiteralPath (Join-Path $msvcRoot.FullName "bin\Hostx64\arm64\cl.exe")) -and (Test-Path -LiteralPath (Join-Path $msvcRoot.FullName "lib\arm64\libcmt.lib")))
$clangPath = Join-Path $vsRoot "VC\Tools\Llvm\x64\bin\clang.exe"
if (-not (Test-Path -LiteralPath $vsDevCmd) -or -not $arm64ToolsPresent -or -not (Test-Path -LiteralPath $clangPath)) {
  $vsInstaller = Join-Path $downloads "vs_BuildTools.exe"
  if (-not (Test-Path -LiteralPath $vsInstaller)) {
    Invoke-WebRequest -UseBasicParsing -Uri "https://aka.ms/vs/17/release/vs_BuildTools.exe" -OutFile $vsInstaller
  }
  $signature = Get-AuthenticodeSignature -LiteralPath $vsInstaller
  if ($signature.Status -ne "Valid" -or $signature.SignerCertificate.Subject -notmatch "Microsoft Corporation") {
    throw "Visual Studio Build Tools bootstrapper signature is not valid Microsoft code"
  }
  $arguments = @(
    "--quiet",
    "--wait",
    "--norestart",
    "--nocache",
    "--installPath", $vsRoot,
    "--add", "Microsoft.VisualStudio.Workload.VCTools",
    "--add", "Microsoft.VisualStudio.Component.VC.Tools.ARM64",
    "--add", "Microsoft.VisualStudio.ComponentGroup.NativeDesktop.Llvm.Clang",
    "--includeRecommended"
  )
  $process = Start-Process -FilePath $vsInstaller -ArgumentList $arguments -Wait -PassThru
  if ($process.ExitCode -notin @(0,3010)) {
    throw "Visual Studio Build Tools installer exited $($process.ExitCode)"
  }
}
$msvcRoot = Get-ChildItem -LiteralPath (Join-Path $vsRoot "VC\Tools\MSVC") -Directory -ErrorAction SilentlyContinue |
  Sort-Object Name -Descending |
  Select-Object -First 1
$arm64ToolsPresent = ($null -ne $msvcRoot -and (Test-Path -LiteralPath (Join-Path $msvcRoot.FullName "bin\Hostx64\arm64\cl.exe")) -and (Test-Path -LiteralPath (Join-Path $msvcRoot.FullName "lib\arm64\libcmt.lib")))
if (-not $arm64ToolsPresent) { throw "Visual Studio ARM64 C++ tools are missing" }
$clangPath = Join-Path $vsRoot "VC\Tools\Llvm\x64\bin\clang.exe"
if (-not (Test-Path -LiteralPath $clangPath)) { throw "Visual Studio LLVM Clang is missing" }

$env:Path = @(
  $nodeHome,
  (Join-Path $gitHome "cmd"),
  (Join-Path $env:CARGO_HOME "bin"),
  $env:Path
) -join ";"
$msvcDirectories = @(Get-ChildItem -LiteralPath (Join-Path $vsRoot "VC\Tools\MSVC") -Directory -ErrorAction SilentlyContinue)
$rustcVerbose = @(& (Join-Path $env:CARGO_HOME "bin\rustc.exe") -vV)
$rustcHost = ($rustcVerbose | Where-Object { $_ -like "host:*" } | Select-Object -First 1).Substring(5).Trim()
$payload = [ordered]@{
  platform = "windows"
  architecture = $env:PROCESSOR_ARCHITECTURE
  node_version = (& $node --version)
  node_archive_sha256 = $actualNodeHash
  pnpm_version = (& $corepack "pnpm@10" "--version")
  git_version = (& (Join-Path $gitHome "cmd\git.exe") --version)
  git_archive_sha256 = $actualGitHash
  rustc_version = (& (Join-Path $env:CARGO_HOME "bin\rustc.exe") --version)
  cargo_version = (& (Join-Path $env:CARGO_HOME "bin\cargo.exe") --version)
  rustup_init_sha256 = $actualRustupHash
  visual_studio_build_tools = Test-Path -LiteralPath $vsDevCmd
  msvc_toolset_count = $msvcDirectories.Count
  msvc_arm64_tools = $arm64ToolsPresent
  clang_path = $clangPath
  rustc_host = $rustcHost
  source_root = $sourceRoot
  source_sha256 = $sourceSha256
  source_present = Test-Path -LiteralPath (Join-Path $sourceRoot "Cargo.toml")
  reboot_required = Test-Path -LiteralPath "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending"
}
`;
}
