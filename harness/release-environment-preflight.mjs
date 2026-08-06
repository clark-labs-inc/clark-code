#!/usr/bin/env node

import { chmodSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const DEFAULT_OUT = path.join(
  repoDir,
  "target",
  "release-preflight",
  `${new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z")}-${process.pid}`,
);
const DEFAULT_VMS = {
  windows: process.env.CLARK_WINDOWS_QA_VM_NAME || "Clark QA - Windows 11 ARM",
  ubuntu: process.env.CLARK_UBUNTU_QA_VM_NAME || "Clark QA - Ubuntu 24.04 Desktop",
};
const REQUIREMENTS = new Set(["clark", "scientist", "utm", "ssh", "remote-cpu"]);
const REMOTE_CREDENTIAL_NAMES = new Set([
  "CLARK_CODE_API_KEY",
  "CLARK_API_KEY",
  "OPENROUTER_API_KEY",
]);
const ENV_NAMES = [
  "CLARK_CODE_API_KEY",
  "CLARK_CODE_PROVIDER",
  "CLARK_CODE_BASE_URL",
  "CLARK_CODE_MODEL",
  "OPENROUTER_API_KEY",
  "CLARK_API_KEY",
  "CLARK_QA_VM_USERNAME",
  "CLARK_QA_VM_PASSWORD",
  "CLARK_QA_AUTH_NAME",
  "CLARK_QA_AUTH_EMAIL",
  "CLARK_QA_AUTH_PASSWORD",
  "CLARK_REMOTE_CPU_LIVE",
  "CLARK_REMOTE_CPU_PAID",
  "CLARK_REMOTE_CPU_HOST",
  "CLARK_REMOTE_CPU_ROOT",
  "CLARK_REMOTE_CPU_TRAJECTORY",
  "CLARK_REMOTE_CPU_WORKER",
  "CLARK_REMOTE_CPU_CREDENTIAL_ENV",
  "CLARK_REMOTE_CPU_MODEL",
  "CLARK_REMOTE_CPU_BASE_URL",
  "CLARK_REMOTE_CPU_RECEIPT",
];

function usage() {
  console.log(`Usage: node harness/release-environment-preflight.mjs [options]

Options:
  --all                         Require every paid, UTM, SSH, and remote lane
  --require NAME                Require clark|scientist|utm|ssh|remote-cpu (repeatable)
  --out PATH                    Write the redacted receipt to PATH
  --exec COMMAND [ARGS...]      Run a command with resolved credentials after checks pass
  --help                        Show this help

The checker reads process variables first, then the ignored Desktop .env,
../clark/.env, and ../clark-scientist/.env. It never prints or persists secret values.`);
}

export function parseDotEnv(source) {
  const values = {};
  for (const rawLine of String(source).split(/\r?\n/)) {
    const match = rawLine.match(/^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=(.*)$/);
    if (!match || !ENV_NAMES.includes(match[1])) continue;
    let value = match[2].trim().replace(/\r$/, "");
    if (
      value.length >= 2
      && ((value.startsWith('"') && value.endsWith('"'))
        || (value.startsWith("'") && value.endsWith("'")))
    ) {
      value = value.slice(1, -1);
    }
    values[match[1]] = value;
  }
  return values;
}

function envSources() {
  return [
    { id: "process", path: null, values: process.env, mode: null },
    { id: "desktop", path: process.env.CLARK_DESKTOP_ENV || path.join(repoDir, ".env") },
    { id: "clark", path: process.env.CLARK_PLATFORM_ENV || path.join(repoDir, "..", "clark", ".env") },
    {
      id: "scientist",
      path: process.env.CLARK_SCIENTIST_ENV || path.join(repoDir, "..", "clark-scientist", ".env"),
    },
  ].map((source) => {
    if (!source.path) return source;
    try {
      const metadata = statSync(source.path);
      return {
        ...source,
        values: parseDotEnv(readFileSync(source.path, "utf8")),
        mode: metadata.mode & 0o777,
        exists: true,
      };
    } catch {
      return { ...source, values: {}, mode: null, exists: false };
    }
  });
}

export function resolveValue(name, sources) {
  for (const source of sources) {
    const value = source.values?.[name];
    if (typeof value === "string" && value.trim()) {
      return { value, source: source.id, path: source.path, mode: source.mode };
    }
  }
  return { value: "", source: null, path: null, mode: null };
}

function secureSource(value) {
  return value.source === "process" || (value.mode != null && (value.mode & 0o077) === 0);
}

function commandPath(command) {
  const known = command === "utmctl"
    ? ["/opt/homebrew/bin/utmctl", "/Applications/UTM.app/Contents/MacOS/utmctl"]
    : [];
  for (const candidate of known) {
    try {
      if (statSync(candidate).isFile()) return candidate;
    } catch {
      // Keep looking.
    }
  }
  const result = spawnSync("/bin/sh", ["-lc", `command -v ${command}`], {
    encoding: "utf8",
    timeout: 5_000,
  });
  return result.status === 0 ? result.stdout.trim() : "";
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    timeout: options.timeoutMs ?? 15_000,
    maxBuffer: 2 * 1024 * 1024,
    env: options.env,
  });
  return {
    ok: result.status === 0,
    code: result.status,
    stdout: result.stdout || "",
    stderr: result.stderr || result.error?.message || "",
  };
}

export function parseUtmList(source) {
  return String(source)
    .split(/\r?\n/)
    .slice(1)
    .flatMap((line) => {
      const match = line.match(
        /^([0-9A-Fa-f]{8}(?:-[0-9A-Fa-f]{4}){3}-[0-9A-Fa-f]{12})\s+(\S+)\s+(.+?)\s*$/,
      );
      return match
        ? [{ uuid: match[1].toUpperCase(), status: match[2], name: match[3] }]
        : [];
    });
}

function check(checks, id, required, ok, reason, details = {}) {
  checks.push({
    id,
    required,
    status: ok ? "passed" : required ? "blocked" : "not_required",
    reason: ok ? null : reason,
    ...details,
  });
}

function requiredFor(requirements, name) {
  return requirements.has(name);
}

function credentialCheck(checks, id, required, resolved, reason) {
  check(checks, id, required, Boolean(resolved.value) && secureSource(resolved),
    resolved.value ? "secret_file_permissions" : reason, {
      source: resolved.source,
      configured: Boolean(resolved.value),
      secure_file_mode: resolved.value ? secureSource(resolved) : null,
    });
}

function sourceReceipt(sources) {
  return sources
    .filter((source) => source.path)
    .map((source) => ({
      id: source.id,
      path: source.path,
      exists: Boolean(source.exists),
      mode: source.mode == null ? null : source.mode.toString(8).padStart(3, "0"),
      secure_mode: source.mode == null ? null : (source.mode & 0o077) === 0,
    }));
}

function buildReceipt(requirements, sources) {
  const checks = [];
  const value = (name) => resolveValue(name, sources);
  const clarkKey = value("CLARK_CODE_API_KEY");
  const openrouterKey = value("OPENROUTER_API_KEY");
  const syncKey = value("CLARK_API_KEY").value ? value("CLARK_API_KEY") : clarkKey;

  credentialCheck(checks, "credential.clark_platform", requiredFor(requirements, "clark"), clarkKey,
    "missing_clark_code_api_key");
  const provider = value("CLARK_CODE_PROVIDER").value || "clark-platform";
  const baseUrl = value("CLARK_CODE_BASE_URL").value || "https://api.clarkslabs.com/v1";
  const model = value("CLARK_CODE_MODEL").value || "qwen/qwen3.7-flash";
  check(checks, "config.clark_paid_route", requiredFor(requirements, "clark"),
    provider === "clark-platform" && baseUrl === "https://api.clarkslabs.com/v1" && model === "qwen/qwen3.7-flash",
    "clark_paid_route_mismatch", { provider, base_url: baseUrl, model });

  credentialCheck(checks, "credential.openrouter", requiredFor(requirements, "scientist"), openrouterKey,
    "missing_openrouter_api_key");
  credentialCheck(checks, "credential.specialist_sync", requiredFor(requirements, "scientist"), syncKey,
    "missing_clark_sync_api_key");
  check(checks, "tool.scientist_runner", requiredFor(requirements, "scientist"),
    statSyncSafe(path.join(repoDir, "..", "clark-scientist", "script", "run_qwen_specialist_product_eval.sh")),
    "scientist_runner_missing");

  const tools = {};
  for (const name of ["node", "cargo", "jq", "ssh"]) tools[name] = commandPath(name);
  tools.utmctl = commandPath("utmctl");
  for (const [name, found] of Object.entries(tools)) {
    const required = name === "utmctl" ? requiredFor(requirements, "utm") : true;
    check(checks, `tool.${name}`, required, Boolean(found), "tool_unavailable", { path: found || null });
  }

  const utmList = tools.utmctl ? run(tools.utmctl, ["list"]) : { ok: false, stdout: "", stderr: "utmctl unavailable" };
  const guests = parseUtmList(utmList.stdout);
  for (const [platform, name] of Object.entries(DEFAULT_VMS)) {
    const guest = guests.find((entry) => entry.name === name);
    check(checks, `utm.${platform}`, requiredFor(requirements, "utm"), Boolean(guest),
      "utm_vm_not_registered", { vm_name: name, status: guest?.status || null });
  }
  for (const name of ["CLARK_QA_VM_USERNAME", "CLARK_QA_VM_PASSWORD", "CLARK_QA_AUTH_NAME", "CLARK_QA_AUTH_EMAIL", "CLARK_QA_AUTH_PASSWORD"]) {
    credentialCheck(checks, `credential.${name.toLowerCase()}`, requiredFor(requirements, "utm"), value(name), `missing_${name.toLowerCase()}`);
  }

  const host = value("CLARK_REMOTE_CPU_HOST").value || "nucleus";
  const sshConfig = run(tools.ssh || "ssh", ["-G", host], { timeoutMs: 5_000 });
  const sshPing = run(tools.ssh || "ssh", ["-o", "BatchMode=yes", "-o", "ConnectTimeout=5", host, "true"], { timeoutMs: 8_000 });
  check(checks, "ssh.host_config", requiredFor(requirements, "ssh"), sshConfig.ok, "ssh_host_not_configured", { host });
  check(checks, "ssh.reachable", requiredFor(requirements, "ssh"), sshPing.ok, "ssh_host_unreachable", { host });

  const remoteRequired = requiredFor(requirements, "remote-cpu");
  const remoteLive = value("CLARK_REMOTE_CPU_LIVE");
  const remotePaid = value("CLARK_REMOTE_CPU_PAID");
  check(checks, "remote.clark_remote_cpu_live", remoteRequired, remoteLive.value === "1",
    "remote_live_not_enabled", { configured: Boolean(remoteLive.value), enabled: remoteLive.value === "1" });
  check(checks, "remote.clark_remote_cpu_paid", remoteRequired, remotePaid.value === "1",
    "remote_paid_not_enabled", { configured: Boolean(remotePaid.value), enabled: remotePaid.value === "1" });

  const remoteRoot = value("CLARK_REMOTE_CPU_ROOT").value || "/tmp/clark-code-remote-smoke";
  const remoteTrajectory = value("CLARK_REMOTE_CPU_TRAJECTORY").value || "/tmp/clark-code-remote-trajectory";
  const remoteBaseUrl = value("CLARK_REMOTE_CPU_BASE_URL").value || "https://api.clarkslabs.com/v1";
  check(checks, "remote.route", remoteRequired, remoteBaseUrl.startsWith("https://"),
    "remote_base_url_must_be_https", { base_url: remoteBaseUrl });
  check(checks, "remote.paths_are_absolute", remoteRequired,
    path.isAbsolute(remoteRoot) && path.isAbsolute(remoteTrajectory),
    "remote_paths_must_be_absolute", { root: remoteRoot, trajectory: remoteTrajectory });

  const remoteWorker = value("CLARK_REMOTE_CPU_WORKER");
  check(checks, "remote.worker_path", remoteRequired,
    path.isAbsolute(remoteWorker.value) && statSyncSafe(remoteWorker.value),
    remoteWorker.value ? "remote_worker_missing" : "missing_clark_remote_cpu_worker",
    { configured: Boolean(remoteWorker.value), absolute: path.isAbsolute(remoteWorker.value), path: remoteWorker.value || null });

  const remoteCredentialName = value("CLARK_REMOTE_CPU_CREDENTIAL_ENV");
  const remoteCredential = remoteCredentialName.value ? value(remoteCredentialName.value) : { value: "", source: null, mode: null };
  check(checks, "remote.credential_env", remoteRequired,
    REMOTE_CREDENTIAL_NAMES.has(remoteCredentialName.value) && Boolean(remoteCredential.value) && secureSource(remoteCredential),
    remoteCredentialName.value ? "remote_credential_unavailable_or_insecure" : "missing_clark_remote_cpu_credential_env",
    {
      configured: Boolean(remoteCredentialName.value),
      name: remoteCredentialName.value || null,
      secure_file_mode: remoteCredential.value ? secureSource(remoteCredential) : null,
    });

  const remoteModel = value("CLARK_REMOTE_CPU_MODEL");
  check(checks, "remote.model", remoteRequired, Boolean(remoteModel.value), "missing_clark_remote_cpu_model",
    { configured: Boolean(remoteModel.value), model: remoteModel.value || null });
  const remoteReceipt = value("CLARK_REMOTE_CPU_RECEIPT");
  check(checks, "remote.receipt", remoteRequired, path.isAbsolute(remoteReceipt.value),
    remoteReceipt.value ? "remote_receipt_must_be_absolute" : "missing_clark_remote_cpu_receipt",
    { configured: Boolean(remoteReceipt.value), absolute: path.isAbsolute(remoteReceipt.value), path: remoteReceipt.value || null });

  const blocked = checks.filter((item) => item.status === "blocked");
  return {
    schema_version: 1,
    benchmark: "clark_desktop_release_environment_preflight",
    generated_at: new Date().toISOString(),
    requirements: [...requirements].sort(),
    status: blocked.length === 0 ? "passed" : "blocked",
    sources: sourceReceipt(sources),
    checks,
    summary: {
      total: checks.length,
      passed: checks.filter((item) => item.status === "passed").length,
      blocked: blocked.length,
      not_required: checks.filter((item) => item.status === "not_required").length,
    },
  };
}

function statSyncSafe(filePath) {
  try {
    return statSync(filePath).isFile();
  } catch {
    return false;
  }
}

function parseArgs(argv) {
  const requirements = new Set();
  let out = DEFAULT_OUT;
  let exec = null;
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--help" || arg === "-h") return { help: true };
    if (arg === "--all") {
      for (const name of REQUIREMENTS) requirements.add(name);
      continue;
    }
    if (arg === "--require") {
      const name = argv[++index];
      if (!REQUIREMENTS.has(name)) throw new Error(`unknown requirement ${name}`);
      requirements.add(name);
      continue;
    }
    if (arg === "--out") {
      out = path.resolve(repoDir, argv[++index]);
      continue;
    }
    if (arg === "--exec") {
      exec = argv.slice(index + 1);
      break;
    }
    throw new Error(`unknown argument ${arg}`);
  }
  return { requirements, out, exec };
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    usage();
    return 0;
  }
  const sources = envSources();
  const receipt = buildReceipt(options.requirements, sources);
  mkdirSync(options.out, { recursive: true, mode: 0o700 });
  chmodSync(options.out, 0o700);
  const receiptPath = path.join(options.out, "receipt.json");
  writeFileSync(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o600 });
  console.log(JSON.stringify({ status: receipt.status, receipt: receiptPath, summary: receipt.summary }));
  for (const item of receipt.checks.filter((entry) => entry.status === "blocked")) {
    console.error(`BLOCKED ${item.id}: ${item.reason}`);
  }
  if (receipt.status === "blocked" || !options.exec) return receipt.status === "blocked" ? 2 : 0;
  const [command, ...args] = options.exec;
  const resolved = { ...process.env };
  for (const source of sources.slice(1)) {
    for (const [name, value] of Object.entries(source.values || {})) {
      if (!resolved[name]) resolved[name] = value;
    }
  }
  const child = spawnSync(command, args, {
    stdio: "inherit",
    env: { ...process.env, ...resolved, CLARK_API_KEY: resolved.CLARK_API_KEY || resolved.CLARK_CODE_API_KEY },
  });
  return child.status ?? 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    process.exitCode = main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 2;
  }
}
