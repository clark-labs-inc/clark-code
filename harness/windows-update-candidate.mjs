#!/usr/bin/env node

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
import { pathToFileURL } from "node:url";

const SEED_VERSION = "0.0.0";
const WINDOWS_PLATFORM = "windows-x86_64";
const WINDOWS_ASSET = "ClarkCode_x64-setup.exe";
const MANIFEST_NAME = "windows-update.json";

function exactTag(value) {
  if (!/^v\d+\.\d+\.\d+$/.test(value)) {
    throw new Error("release tag must be vX.Y.Z");
  }
  return value;
}

function exactRevision(value) {
  if (!/^[0-9a-f]{40}$/.test(value)) {
    throw new Error("source revision must be a clean 40-character Git revision");
  }
  return value;
}

function exactDate(value) {
  if (
    typeof value !== "string"
    || Number.isNaN(Date.parse(value))
    || new Date(value).toISOString() !== value
  ) {
    throw new Error("published-at must be an exact ISO-8601 UTC timestamp");
  }
  return value;
}

function normalizeBase(value) {
  const url = new URL(value);
  if (url.protocol !== "https:") {
    throw new Error("update candidate base URL must use HTTPS");
  }
  return url.href.replace(/\/+$/, "");
}

export function buildWindowsUpdateCandidate({
  tag,
  baseUrl,
  sourceRevision,
  publishedAt,
  updaterFragment,
}) {
  exactTag(tag);
  exactRevision(sourceRevision);
  exactDate(publishedAt);
  const normalizedBase = normalizeBase(baseUrl);
  const entry = updaterFragment?.platforms?.[WINDOWS_PLATFORM];
  const artifactUrl =
    `${normalizedBase}/releases/${tag}/${WINDOWS_ASSET}`;
  if (
    typeof entry?.signature !== "string"
    || entry.signature.trim().length < 32
    || entry.url !== artifactUrl
    || Object.keys(updaterFragment?.platforms || {}).length !== 1
  ) {
    throw new Error("Windows updater fragment is missing its exact signed immutable artifact");
  }
  const endpoint =
    `${normalizedBase}/releases/${tag}/${MANIFEST_NAME}`;
  const manifest = {
    version: tag.slice(1),
    notes: `Clark Code ${tag}`,
    pub_date: publishedAt,
    platforms: {
      [WINDOWS_PLATFORM]: {
        signature: entry.signature.trim(),
        url: artifactUrl,
      },
    },
  };
  const manifestBytes = `${JSON.stringify(manifest, null, 2)}\n`;
  const manifestSha256 = createHash("sha256")
    .update(manifestBytes)
    .digest("hex");
  const seedConfig = {
    version: SEED_VERSION,
    bundle: {
      createUpdaterArtifacts: false,
    },
    plugins: {
      updater: {
        endpoints: [endpoint],
      },
    },
  };
  const receipt = {
    schema_version: 1,
    receipt_type: "clark_code_windows_update_candidate",
    status: "passed",
    source_revision: sourceRevision,
    tag,
    version: tag.slice(1),
    seed_version: SEED_VERSION,
    endpoint,
    artifact_url: artifactUrl,
    manifest_sha256: manifestSha256,
    signer: "tauri_ed25519",
  };
  return { manifest, manifestBytes, seedConfig, receipt };
}

export function validateWindowsUpdateCandidateReceipt(receipt, expectedRevision) {
  exactRevision(expectedRevision);
  if (
    receipt?.receipt_type !== "clark_code_windows_update_candidate"
    || receipt?.status !== "passed"
    || receipt?.source_revision !== expectedRevision
    || !/^v\d+\.\d+\.\d+$/.test(receipt?.tag || "")
    || receipt?.version !== receipt.tag.slice(1)
    || receipt?.seed_version !== SEED_VERSION
    || receipt?.endpoint
      !== `https://downloads.clarkchat.com/desktop/releases/${receipt.tag}/${MANIFEST_NAME}`
    || receipt?.artifact_url
      !== `https://downloads.clarkchat.com/desktop/releases/${receipt.tag}/${WINDOWS_ASSET}`
    || !/^[0-9a-f]{64}$/.test(receipt?.manifest_sha256 || "")
    || receipt?.signer !== "tauri_ed25519"
  ) {
    throw new Error("Windows update candidate receipt is missing, stale, or malformed");
  }
  return receipt;
}

export async function verifyWindowsUpdateCandidateEndpoint(receipt) {
  validateWindowsUpdateCandidateReceipt(receipt, receipt.source_revision);
  const response = await fetch(receipt.endpoint, {
    headers: {
      "cache-control": "no-cache, no-store, max-age=0",
      pragma: "no-cache",
    },
  });
  if (!response.ok) {
    throw new Error(`Windows update candidate returned HTTP ${response.status}`);
  }
  const bytes = Buffer.from(await response.arrayBuffer());
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  const size = Number(response.headers.get("content-length"));
  if (
    sha256 !== receipt.manifest_sha256
    || size !== bytes.length
    || response.headers.get("x-amz-meta-sha256") !== receipt.manifest_sha256
    || response.headers.get("x-amz-meta-clark-version") !== receipt.tag
    || response.headers.get("x-amz-meta-source-revision")
      !== receipt.source_revision
  ) {
    throw new Error("public Windows update candidate identity does not match its receipt");
  }
  const manifest = JSON.parse(bytes.toString("utf8"));
  const platform = manifest?.platforms?.[WINDOWS_PLATFORM];
  if (
    manifest?.version !== receipt.version
    || Object.keys(manifest?.platforms || {}).length !== 1
    || platform?.url !== receipt.artifact_url
    || typeof platform?.signature !== "string"
    || platform.signature.length < 32
  ) {
    throw new Error("public Windows update candidate manifest is malformed");
  }
  return {
    url: receipt.endpoint,
    sha256,
    size: bytes.length,
    version: manifest.version,
    artifact_url: platform.url,
  };
}

export function writeWindowsUpdateCandidate({
  tag,
  baseUrl,
  sourceRevision,
  publishedAt,
  updaterFragmentPath,
  outputDir,
}) {
  if (existsSync(outputDir)) {
    throw new Error(`refusing to overwrite existing output directory ${outputDir}`);
  }
  const result = buildWindowsUpdateCandidate({
    tag,
    baseUrl,
    sourceRevision,
    publishedAt,
    updaterFragment: JSON.parse(readFileSync(updaterFragmentPath, "utf8")),
  });
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  chmodSync(outputDir, 0o700);
  const manifestPath = path.join(outputDir, MANIFEST_NAME);
  const seedConfigPath = path.join(outputDir, "seed-tauri.conf.json");
  const receiptPath = path.join(outputDir, "receipt.json");
  writeFileSync(manifestPath, result.manifestBytes, { mode: 0o600 });
  writeFileSync(
    seedConfigPath,
    `${JSON.stringify(result.seedConfig, null, 2)}\n`,
    { mode: 0o600 },
  );
  writeFileSync(
    receiptPath,
    `${JSON.stringify(result.receipt, null, 2)}\n`,
    { mode: 0o600 },
  );
  for (const file of [manifestPath, seedConfigPath, receiptPath]) {
    chmodSync(file, 0o600);
  }
  return { ...result, manifestPath, seedConfigPath, receiptPath };
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

function main() {
  const args = process.argv.slice(2);
  const result = writeWindowsUpdateCandidate({
    tag: valueArg(args, "--tag"),
    baseUrl: valueArg(args, "--base-url"),
    sourceRevision: valueArg(args, "--source-revision"),
    publishedAt: valueArg(args, "--published-at"),
    updaterFragmentPath: path.resolve(valueArg(args, "--updater-fragment")),
    outputDir: path.resolve(valueArg(args, "--out")),
  });
  process.stdout.write(`${JSON.stringify({
    status: result.receipt.status,
    manifest: result.manifestPath,
    seed_config: result.seedConfigPath,
    receipt: result.receiptPath,
  }, null, 2)}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`${error.stack || error.message}\n`);
    process.exitCode = 1;
  }
}
