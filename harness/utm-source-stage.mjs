#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  accessSync,
  chmodSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

import { executeGuestJson } from "./utm-guest-channel.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const GUESTS = {
  windows: {
    vm_name: "Clark QA - Windows 11 ARM",
    archive: (shortHash) => String.raw`C:\Users\Public\clark-source-${shortHash}.tgz`,
  },
  ubuntu: {
    vm_name: "Clark QA - Ubuntu 24.04 Desktop",
    archive: (shortHash) => `/var/tmp/clark-source-${shortHash}.tgz`,
  },
};

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

function windowsReserved(segment) {
  const stem = segment.split(".", 1)[0].toUpperCase();
  return (
    ["CON", "PRN", "AUX", "NUL"].includes(stem)
    || /^COM[1-9]$/.test(stem)
    || /^LPT[1-9]$/.test(stem)
  );
}

export function validateSourcePaths(paths) {
  const accepted = [];
  const excluded = [];
  for (const relative of paths) {
    if (!relative || path.isAbsolute(relative) || relative.split("/").includes("..")) {
      throw new Error(`unsafe source path ${JSON.stringify(relative)}`);
    }
    if (relative.includes("\\") || relative.includes("\n") || relative.includes("\r")) {
      throw new Error(`source path is not portable to Windows: ${JSON.stringify(relative)}`);
    }
    const segments = relative.split("/");
    if (segments.some(windowsReserved)) {
      if (relative === "NUL") {
        excluded.push({ path: relative, reason: "Windows-reserved untracked artifact" });
        continue;
      }
      throw new Error(`source path uses a Windows-reserved name: ${relative}`);
    }
    const basename = segments.at(-1);
    if (
      basename === ".env"
      || (basename.startsWith(".env.") && !basename.endsWith(".example"))
    ) {
      throw new Error(`credential-bearing environment file entered the source set: ${relative}`);
    }
    const metadata = lstatSync(path.join(repoDir, relative));
    if (!metadata.isFile()) {
      throw new Error(`source entry must be a regular file: ${relative}`);
    }
    accepted.push(relative);
  }
  accepted.sort();
  if (new Set(accepted).size !== accepted.length) {
    throw new Error("source set contains duplicate paths");
  }
  for (const required of [
    "Cargo.toml",
    "app/package.json",
    "harness/clark-code-feature-map.json",
    "harness/utm-source-stage.mjs",
  ]) {
    if (!accepted.includes(required)) throw new Error(`source set is missing ${required}`);
  }
  return { accepted, excluded };
}

function gitSourcePaths() {
  const listed = run(
    "git",
    ["ls-files", "--cached", "--others", "--exclude-standard", "-z"],
    { binary_output: true },
  );
  if (!listed.ok) throw new Error(`cannot enumerate source files: ${listed.stderr}`);
  const paths = listed.stdout
    .toString("utf8")
    .split("\0")
    .filter(Boolean);
  return validateSourcePaths(paths);
}

function sourceRevision() {
  const revision = run("git", ["rev-parse", "HEAD"]);
  if (!revision.ok) return "unknown";
  const status = run("git", ["status", "--porcelain"]);
  return `${revision.stdout.trim()}${status.stdout.trim() ? "-dirty" : ""}`;
}

function sha256File(filePath) {
  return createHash("sha256").update(readFileSync(filePath)).digest("hex");
}

export function createSourcePackage(outputDir) {
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  chmodSync(outputDir, 0o700);
  const sourceSet = gitSourcePaths();
  const archivePath = path.join(outputDir, "clark-desktop-source.tgz");
  const fileList = Buffer.from(`${sourceSet.accepted.join("\0")}\0`);
  const archived = run(
    "tar",
    ["-czf", archivePath, "--null", "-T", "-"],
    {
      input: fileList,
      env: { ...process.env, COPYFILE_DISABLE: "1" },
    },
  );
  if (!archived.ok) throw new Error(`cannot create source archive: ${archived.stderr}`);
  const members = run("tar", ["-tzf", archivePath]);
  if (!members.ok) throw new Error(`cannot inspect source archive: ${members.stderr}`);
  const metadataEntries = members.stdout.split(/\r?\n/).filter((entry) => (
    entry.split("/").some((segment) => segment.startsWith("._"))
    || entry.split("/").includes("__MACOSX")
  ));
  if (metadataEntries.length) {
    throw new Error(`source archive contains macOS metadata: ${metadataEntries[0]}`);
  }
  chmodSync(archivePath, 0o600);
  const archiveSha256 = sha256File(archivePath);
  return {
    archivePath,
    archiveSha256,
    sourceRevision: sourceRevision(),
    fileCount: sourceSet.accepted.length,
    excluded: sourceSet.excluded,
  };
}

function powershellLiteral(value) {
  return `'${String(value).replaceAll("'", "''")}'`;
}

export function windowsExtractProbe({ archivePath, sourceSha256 }) {
  const shortHash = sourceSha256.slice(0, 12);
  const root = String.raw`C:\ClarkQA\source-${shortHash}`;
  return String.raw`
$expected = ${powershellLiteral(sourceSha256)}
$archive = ${powershellLiteral(archivePath)}
$root = ${powershellLiteral(root)}
$actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLower()
if ($actual -ne $expected) { throw "source archive SHA-256 mismatch" }
$marker = Join-Path $root ".source-sha256"
$reused = (Test-Path -LiteralPath $marker) -and (
  (Get-Content -LiteralPath $marker -Raw).Trim() -eq $expected
)
if ((Test-Path -LiteralPath $root) -and -not $reused) {
  $quarantine = $root + "-incomplete-" + [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
  Move-Item -LiteralPath $root -Destination $quarantine
}
if (-not $reused) {
  New-Item -ItemType Directory -Force -Path $root | Out-Null
  & "$env:SystemRoot\System32\tar.exe" -xzf $archive -C $root
  if ($LASTEXITCODE -ne 0) { throw "Windows source extraction failed" }
  [IO.File]::WriteAllText($marker, $expected + [Environment]::NewLine)
}
[IO.File]::WriteAllText(
  "C:\ClarkQA\source-current.txt",
  $root + [Environment]::NewLine
)
$payload = [ordered]@{
  archive_sha256 = $actual
  source_root = $root
  source_sha256 = (Get-Content -LiteralPath $marker -Raw).Trim()
  reused = $reused
  file_count = @(Get-ChildItem -LiteralPath $root -File -Recurse).Count
  appledouble_count = @(Get-ChildItem -LiteralPath $root -File -Recurse -Filter "._*").Count
  env_present = Test-Path -LiteralPath (Join-Path $root ".env")
  cargo_toml_present = Test-Path -LiteralPath (Join-Path $root "Cargo.toml")
  pointer_written = (
    (Get-Content -LiteralPath "C:\ClarkQA\source-current.txt" -Raw).Trim() -eq $root
  )
}
`;
}

export function ubuntuExtractProbe({ archivePath, sourceSha256 }) {
  const shortHash = sourceSha256.slice(0, 12);
  const root = `/opt/clark-qa/source-${shortHash}`;
  return `import hashlib, pathlib, tarfile, time
expected = "${sourceSha256}"
archive = pathlib.Path("${archivePath}")
root = pathlib.Path("${root}")
actual = hashlib.sha256(archive.read_bytes()).hexdigest()
if actual != expected:
    raise RuntimeError("source archive SHA-256 mismatch")
marker = root / ".source-sha256"
reused = root.is_dir() and marker.is_file() and marker.read_text().strip() == expected
if root.exists() and not reused:
    root.rename(root.with_name(root.name + "-incomplete-" + str(int(time.time()))))
if not reused:
    root.mkdir(parents=True)
    with tarfile.open(archive, "r:gz") as bundle:
        canonical_root = root.resolve()
        for member in bundle.getmembers():
            candidate = (root / member.name).resolve()
            if candidate != canonical_root and canonical_root not in candidate.parents:
                raise RuntimeError("source archive path escape")
            if member.issym() or member.islnk():
                raise RuntimeError("source archive link entry")
        bundle.extractall(root, filter="data")
    marker.write_text(expected + "\\n")
pointer = pathlib.Path("/opt/clark-qa/source-current.txt")
pointer.write_text(str(root) + "\\n")
payload = {
    "archive_sha256": actual,
    "source_root": str(root),
    "source_sha256": marker.read_text().strip(),
    "reused": reused,
    "file_count": sum(1 for item in root.rglob("*") if item.is_file()),
    "appledouble_count": sum(1 for item in root.rglob("._*") if item.is_file()),
    "env_present": (root / ".env").exists(),
    "cargo_toml_present": (root / "Cargo.toml").is_file(),
    "pointer_written": pointer.read_text().strip() == str(root),
}
`;
}

export function stageSourceToGuest(platform, sourcePackage) {
  const guest = GUESTS[platform];
  if (!guest) throw new Error(`unsupported source staging platform ${platform}`);
  const shortHash = sourcePackage.archiveSha256.slice(0, 12);
  const guestArchive = guest.archive(shortHash);
  const pushed = run(
    "utmctl",
    ["file", "push", guest.vm_name, guestArchive],
    {
      input: readFileSync(sourcePackage.archivePath),
      timeout_ms: 600_000,
    },
  );
  if (!pushed.ok) {
    return {
      platform,
      vm_name: guest.vm_name,
      status: "failed",
      error: pushed.stderr || pushed.stdout || "source transfer failed",
    };
  }
  const extracted = executeGuestJson({
    platform,
    vmName: guest.vm_name,
    state: "started",
    probeSource: platform === "windows"
      ? windowsExtractProbe({
          archivePath: guestArchive,
          sourceSha256: sourcePackage.archiveSha256,
        })
      : ubuntuExtractProbe({
          archivePath: guestArchive,
          sourceSha256: sourcePackage.archiveSha256,
        }),
    run,
    timeoutMs: 300_000,
    pollAttempts: 300,
    executionAttempts: 2,
  });
  const passed = (
    extracted.ok
    && extracted.data.archive_sha256 === sourcePackage.archiveSha256
    && extracted.data.source_sha256 === sourcePackage.archiveSha256
    && extracted.data.cargo_toml_present === true
    && extracted.data.pointer_written === true
    && extracted.data.env_present === false
    && extracted.data.appledouble_count === 0
    && extracted.data.file_count === sourcePackage.fileCount + 1
  );
  return {
    platform,
    vm_name: guest.vm_name,
    status: passed ? "passed" : "failed",
    ...(extracted.ok ? { data: extracted.data } : { error: extracted.error }),
    attempts: extracted.attempts,
  };
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

async function runCli() {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    console.log(`Stage the exact Clark Desktop source in UTM guests

Usage:
  node harness/utm-source-stage.mjs stage --platform windows|ubuntu|all
    [--out NEW_DIRECTORY]

The command packages tracked plus non-ignored untracked files from the current
dirty worktree, rejects secret-bearing and Windows-unsafe paths, SHA-256 pins
the archive, transfers it through the UTM guest agent, and writes a verified
current-source pointer in each guest. It never includes the ignored .env.`);
    return;
  }
  if (args[0] !== "stage") throw new Error(`unknown command ${JSON.stringify(args[0])}`);
  for (let index = 1; index < args.length; index += 1) {
    const arg = args[index];
    if (["--platform", "--out"].includes(arg)) {
      index += 1;
      continue;
    }
    if (["--platform=", "--out="].some((prefix) => arg.startsWith(prefix))) continue;
    throw new Error(`unknown argument ${JSON.stringify(arg)}`);
  }
  const selected = valueArg(args, "--platform") || "all";
  const platforms = selected === "all" ? ["windows", "ubuntu"] : [selected];
  if (platforms.some((platform) => !GUESTS[platform])) {
    throw new Error("--platform must be windows, ubuntu, or all");
  }
  const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
  const outputDir = path.resolve(
    repoDir,
    valueArg(args, "--out")
      || path.join("target", "utm-source-stage", `${stamp}-${process.pid}`),
  );
  try {
    accessSync(outputDir);
    throw new Error(`refusing to overwrite source staging output ${outputDir}`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  const sourcePackage = createSourcePackage(outputDir);
  const guests = platforms.map((platform) => stageSourceToGuest(platform, sourcePackage));
  const receipt = {
    schema_version: 1,
    benchmark: "clark_code_utm_source_stage",
    status: guests.every((guest) => guest.status === "passed") ? "passed" : "failed",
    generated_at: new Date().toISOString(),
    source_revision: sourcePackage.sourceRevision,
    archive: {
      file: path.basename(sourcePackage.archivePath),
      sha256: sourcePackage.archiveSha256,
      file_count: sourcePackage.fileCount,
      excluded: sourcePackage.excluded,
    },
    virtualization: "utm",
    required_user_vm_actions: 0,
    credential_recorded: false,
    ignored_env_included: false,
    guests,
  };
  const receiptPath = path.join(outputDir, "receipt.json");
  writeFileSync(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o600 });
  console.log(JSON.stringify({
    status: receipt.status,
    archive_sha256: receipt.archive.sha256,
    guests: Object.fromEntries(guests.map((guest) => [guest.platform, guest.status])),
    required_user_vm_actions: 0,
  }));
  console.log(`RECEIPT=${receiptPath}`);
  if (receipt.status !== "passed") process.exitCode = 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await runCli();
}
