import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.dirname(harnessDir);
const artifactDir =
  process.env.CLARK_SECURITY_ARTIFACT_DIR
  || path.join("/tmp", "clark-security-simulation");
const paid = process.argv.includes("--paid");
const liveOnly = process.argv.includes("--live-only");
const manifest = JSON.parse(
  await readFile(path.join(harnessDir, "clark-code-feature-map.json"), "utf8"),
);

function redact(text, secrets) {
  let safe = String(text)
    .replace(/\bck_(?:live|test)_[A-Za-z0-9._-]+\b/g, "ck_[REDACTED]")
    .replace(
      /(authorization["']?\s*[:=]\s*["']?bearer\s+)[^\s"',}]+/gi,
      "$1[REDACTED]",
    );
  for (const secret of secrets) {
    if (secret) safe = safe.split(secret).join("[REDACTED]");
  }
  return safe;
}

async function loadPaidEnvironment() {
  const accepted = new Set([
    "CLARK_CODE_API_KEY",
    "CLARK_CODE_PROVIDER",
    "CLARK_CODE_BASE_URL",
    "CLARK_CODE_MODEL",
  ]);
  try {
    const source = await readFile(path.join(repoDir, ".env"), "utf8");
    for (const rawLine of source.split(/\r?\n/)) {
      const line = rawLine.trim();
      if (!line || line.startsWith("#")) continue;
      const separator = line.indexOf("=");
      if (separator < 1) continue;
      const name = line.slice(0, separator).trim();
      if (!accepted.has(name) || process.env[name]) continue;
      let value = line.slice(separator + 1).trim();
      if (
        (value.startsWith("\"") && value.endsWith("\""))
        || (value.startsWith("'") && value.endsWith("'"))
      ) {
        value = value.slice(1, -1);
      }
      if (value) process.env[name] = value;
    }
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  process.env.CLARK_CODE_PROVIDER ||= manifest.live_model.provider;
  process.env.CLARK_CODE_BASE_URL ||= manifest.live_model.base_url;
  process.env.CLARK_CODE_MODEL ||= manifest.live_model.id;
  const errors = [];
  if (!process.env.CLARK_CODE_API_KEY?.trim()) {
    errors.push("CLARK_CODE_API_KEY is missing");
  }
  if (process.env.CLARK_CODE_PROVIDER !== manifest.live_model.provider) {
    errors.push(`CLARK_CODE_PROVIDER must be ${manifest.live_model.provider}`);
  }
  if (process.env.CLARK_CODE_BASE_URL !== manifest.live_model.base_url) {
    errors.push(`CLARK_CODE_BASE_URL must be ${manifest.live_model.base_url}`);
  }
  if (
    process.env.CLARK_CODE_MODEL !== "qwen/qwen3.7-flash"
    || manifest.live_model.upstream_id !== "qwen/qwen3.7-flash"
  ) {
    errors.push(
      "paid Security simulations are locked to qwen/qwen3.7-flash",
    );
  }
  if (errors.length) throw new Error(errors.join("\n"));
}

function run(command, args, env, secrets) {
  return new Promise((resolve) => {
    const startedAt = Date.now();
    let output = "";
    const child = spawn(command, args, {
      cwd: repoDir,
      env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const collect = (chunk) => {
      const safe = redact(chunk.toString(), secrets);
      output = redact(`${output}${safe}`, secrets).slice(-40_000);
      process.stdout.write(safe);
    };
    child.stdout.on("data", collect);
    child.stderr.on("data", collect);
    child.once("close", (code, signal) => resolve({
      command: [command, ...args],
      status: code === 0 ? "passed" : "failed",
      exitCode: code,
      signal,
      durationMs: Date.now() - startedAt,
      outputTail: output.slice(-8_000),
    }));
  });
}

await mkdir(artifactDir, { recursive: true });
const results = [];
const baseEnv = { ...process.env };
const secrets = [];

if (!liveOnly) {
  const offlineSteps = [
    [
      "cargo",
      ["test", "-p", "provider-local", "--test", "security_adversarial", "--", "--nocapture"],
    ],
    [
      "cargo",
      ["test", "-p", "provider-local", "security", "--lib", "--", "--nocapture"],
    ],
    ["corepack", ["pnpm@10", "--dir", "app", "typecheck"]],
    ["corepack", ["pnpm@10", "--dir", "app", "test"]],
    ["corepack", ["pnpm@10", "--dir", "harness", "test:security-ui"]],
  ];
  for (const [command, args] of offlineSteps) {
    process.stdout.write(`\n$ ${[command, ...args].join(" ")}\n`);
    results.push(await run(command, args, baseEnv, secrets));
  }
}

if (paid) {
  await loadPaidEnvironment();
  secrets.push(process.env.CLARK_CODE_API_KEY);
  const env = {
    ...process.env,
    CLARK_CODE_LIVE: "1",
    CLARK_CODE_PROVIDER: manifest.live_model.provider,
    CLARK_CODE_BASE_URL: manifest.live_model.base_url,
    CLARK_CODE_MODEL: manifest.live_model.id,
    CLARK_CODE_MAX_ITERATIONS: "32",
  };
  const args = [
    "test",
    "-p",
    "provider-local",
    "--test",
    "live_clark_code",
    "live_qwen_37_flash_security_adversarial_simulation",
    "--",
    "--ignored",
    "--exact",
    "--nocapture",
    "--test-threads=1",
  ];
  process.stdout.write(`\n$ cargo ${args.join(" ")}\n`);
  results.push(await run("cargo", args, env, secrets));
}

const receipt = {
  simulation: "clark-security-adversarial-v1",
  paid,
  paidModel: paid
    ? {
        alias: manifest.live_model.id,
        label: manifest.live_model.label,
        upstreamId: manifest.live_model.upstream_id,
        provider: manifest.live_model.provider,
      }
    : null,
  fixture: manifest.product,
  results: results.map(({ outputTail: _outputTail, ...result }) => result),
  status: results.every((result) => result.status === "passed") ? "passed" : "failed",
};
const receiptName = paid
  ? (liveOnly ? "receipt-paid.json" : "receipt-full.json")
  : "receipt-offline.json";
const receiptPath = path.join(artifactDir, receiptName);
await writeFile(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`);
console.log(`\n${JSON.stringify({ ...receipt, receiptPath }, null, 2)}`);
if (receipt.status !== "passed") process.exitCode = 1;
