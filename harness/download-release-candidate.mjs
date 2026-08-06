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

import { validateArtifactIdentity } from "./public-release-journey.mjs";

const WINDOWS_ASSET = "ClarkCode_x64-setup.exe";

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

function normalizeBase(value) {
  const url = new URL(value);
  if (url.protocol !== "https:") throw new Error("release candidate base URL must use HTTPS");
  return url.href.replace(/\/+$/, "");
}

export function candidateIdentityFromHeaders(headers, label = "release candidate") {
  const observed = {
    version: headers.get("x-amz-meta-clark-version"),
    sha256: headers.get("x-amz-meta-sha256"),
    size: Number(headers.get("content-length")),
    sourceRevision: headers.get("x-amz-meta-source-revision"),
  };
  if (
    !/^v\d+\.\d+\.\d+$/.test(observed.version || "")
    || !/^[0-9a-f]{64}$/.test(observed.sha256 || "")
    || !Number.isSafeInteger(observed.size)
    || observed.size <= 0
    || !/^[0-9a-f]{40}$/.test(observed.sourceRevision || "")
  ) {
    throw new Error(`${label} returned invalid identity metadata`);
  }
  return observed;
}

export function validateReleaseCandidateDownload(receipt, expectedRevision) {
  exactRevision(expectedRevision);
  const artifact = receipt?.artifact;
  if (
    receipt?.receipt_type !== "clark_code_release_candidate_download"
    || receipt?.status !== "passed"
    || receipt?.source_revision !== expectedRevision
    || !/^v\d+\.\d+\.\d+$/.test(receipt?.tag || "")
    || receipt?.version !== receipt.tag.slice(1)
    || artifact?.asset !== WINDOWS_ASSET
    || artifact?.url !== `${receipt.base_url}/releases/${receipt.tag}/${WINDOWS_ASSET}`
    || artifact?.source_revision !== expectedRevision
    || !/^[0-9a-f]{64}$/.test(artifact?.sha256 || "")
    || !Number.isSafeInteger(artifact?.size)
    || artifact.size <= 0
    || artifact?.file !== WINDOWS_ASSET
    || !/^[0-9a-f]{64}$/.test(receipt?.build_receipt_sha256 || "")
    || typeof receipt?.signer_subject !== "string"
    || receipt.signer_subject.trim().length === 0
    || /[\r\n]/.test(receipt.signer_subject)
    || !/^[0-9A-F]{40}$/.test(receipt?.signer_thumbprint || "")
  ) {
    throw new Error("release candidate download receipt is missing, stale, or malformed");
  }
  return receipt;
}

export function validateWindowsReleaseBuildReceipt(
  receipt,
  expectedRevision,
  expectedTag,
  expectedBaseUrl,
) {
  exactRevision(expectedRevision);
  exactTag(expectedTag);
  const normalizedBase = normalizeBase(expectedBaseUrl);
  const artifact = receipt?.artifact;
  if (
    receipt?.schema_version !== 1
    || receipt?.receipt_type !== "clark_code_windows_release_build"
    || receipt?.status !== "passed"
    || receipt?.source_revision !== expectedRevision
    || receipt?.tag !== expectedTag
    || receipt?.version !== expectedTag.slice(1)
    || typeof receipt?.signer_subject !== "string"
    || receipt.signer_subject.trim().length === 0
    || /[\r\n]/.test(receipt.signer_subject)
    || !/^[0-9A-F]{40}$/.test(receipt?.signer_thumbprint || "")
    || artifact?.asset !== WINDOWS_ASSET
    || artifact?.url !== `${normalizedBase}/releases/${expectedTag}/${WINDOWS_ASSET}`
    || !/^[0-9a-f]{64}$/.test(artifact?.sha256 || "")
    || !Number.isSafeInteger(artifact?.size)
    || artifact.size <= 0
  ) {
    throw new Error("Windows release build receipt is missing, stale, or malformed");
  }
  return receipt;
}

async function fetchExact(url, options = {}) {
  const response = await fetch(url, {
    ...options,
    headers: {
      "cache-control": "no-cache, no-store, max-age=0",
      pragma: "no-cache",
      ...(options.headers || {}),
    },
  });
  if (!response.ok) throw new Error(`${url} returned HTTP ${response.status}`);
  return response;
}

export async function downloadReleaseCandidate({
  tag,
  baseUrl,
  sourceRevision,
  buildReceiptPath,
  outputDir,
}) {
  exactTag(tag);
  exactRevision(sourceRevision);
  const normalizedBase = normalizeBase(baseUrl);
  const buildReceiptBytes = readFileSync(buildReceiptPath);
  const buildReceipt = validateWindowsReleaseBuildReceipt(
    JSON.parse(buildReceiptBytes.toString("utf8")),
    sourceRevision,
    tag,
    normalizedBase,
  );
  if (existsSync(outputDir)) {
    throw new Error(`refusing to overwrite existing output directory ${outputDir}`);
  }
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  chmodSync(outputDir, 0o700);

  const url = `${normalizedBase}/releases/${tag}/${WINDOWS_ASSET}`;
  const head = await fetchExact(url, { method: "HEAD" });
  const expected = candidateIdentityFromHeaders(head.headers, url);
  if (expected.version !== tag) {
    throw new Error(`${url} identifies ${expected.version}, expected ${tag}`);
  }
  if (expected.sourceRevision !== sourceRevision) {
    throw new Error(
      `${url} identifies source revision ${expected.sourceRevision}, `
      + `expected ${sourceRevision}`,
    );
  }
  validateArtifactIdentity(
    expected,
    {
      version: tag,
      sha256: buildReceipt.artifact.sha256,
      size: buildReceipt.artifact.size,
    },
    `${url} build receipt`,
  );
  const response = await fetchExact(url);
  const getIdentity = candidateIdentityFromHeaders(response.headers, url);
  validateArtifactIdentity(getIdentity, expected, `${url} GET`);
  if (getIdentity.sourceRevision !== expected.sourceRevision) {
    throw new Error(`${url} GET source revision differs from HEAD`);
  }
  const bytes = Buffer.from(await response.arrayBuffer());
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  validateArtifactIdentity(
    { version: tag, sha256, size: bytes.length },
    expected,
    `${url} downloaded bytes`,
  );

  const artifactPath = path.join(outputDir, WINDOWS_ASSET);
  writeFileSync(artifactPath, bytes, { mode: 0o600 });
  chmodSync(artifactPath, 0o600);
  const receipt = {
    schema_version: 1,
    receipt_type: "clark_code_release_candidate_download",
    status: "passed",
    generated_at: new Date().toISOString(),
    source_revision: sourceRevision,
    tag,
    version: tag.slice(1),
    base_url: normalizedBase,
    signer_subject: buildReceipt.signer_subject,
    signer_thumbprint: buildReceipt.signer_thumbprint,
    build_receipt_sha256: createHash("sha256")
      .update(buildReceiptBytes)
      .digest("hex"),
    artifact: {
      asset: WINDOWS_ASSET,
      url,
      source_revision: sourceRevision,
      sha256,
      size: bytes.length,
      file: WINDOWS_ASSET,
    },
  };
  const receiptPath = path.join(outputDir, "receipt.json");
  writeFileSync(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, {
    mode: 0o600,
  });
  chmodSync(receiptPath, 0o600);
  return { receipt, receiptPath, artifactPath };
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
  const result = await downloadReleaseCandidate({
    tag: valueArg(args, "--tag"),
    baseUrl: valueArg(args, "--base-url"),
    sourceRevision: valueArg(args, "--source-revision"),
    buildReceiptPath: path.resolve(valueArg(args, "--build-receipt")),
    outputDir: path.resolve(valueArg(args, "--out")),
  });
  process.stdout.write(`${JSON.stringify({
    status: result.receipt.status,
    receipt: result.receiptPath,
    artifact: result.artifactPath,
  }, null, 2)}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    process.stderr.write(`${error.stack || error.message}\n`);
    process.exitCode = 1;
  });
}
