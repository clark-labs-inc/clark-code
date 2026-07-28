#!/usr/bin/env node

import { createHash } from "node:crypto";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const EXPECTED_ALIASES = [
  "ClarkCode.dmg",
  "ClarkCode_x64-setup.exe",
  "ClarkCode_amd64.AppImage",
  "ClarkCode_amd64.deb",
  "ClarkCode_x86_64.rpm",
];

function valueArg(args, name) {
  const inline = args.find((arg) => arg.startsWith(`${name}=`));
  if (inline) return inline.slice(name.length + 1);
  const index = args.indexOf(name);
  if (index < 0 || !args[index + 1] || args[index + 1].startsWith("--")) {
    throw new Error(`${name} requires a value`);
  }
  return args[index + 1];
}

function normalizeBase(value) {
  return value.replace(/\/+$/, "");
}

export function validateArtifactIdentity(observed, expected, label = "release artifact") {
  if (
    observed?.version !== expected?.version
    || observed?.sha256 !== expected?.sha256
    || observed?.size !== expected?.size
  ) {
    throw new Error(
      `${label} identity mismatch: ${JSON.stringify(observed)} `
      + `expected ${JSON.stringify(expected)}`,
    );
  }
  return observed;
}

export function validateS3HeadObject(head, expected) {
  const observed = {
    version: head?.Metadata?.["clark-version"],
    sha256: head?.Metadata?.sha256,
    size: head?.ContentLength,
    sourceRevision: head?.Metadata?.["source-revision"],
  };
  validateArtifactIdentity(observed, {
    version: expected.tag,
    sha256: expected.sha256,
    size: expected.size,
  }, "immutable release object");
  if (
    head?.ContentType !== expected.contentType
    || head?.CacheControl !== expected.cacheControl
  ) {
    throw new Error(
      `immutable release object metadata mismatch: `
      + `${JSON.stringify({
        contentType: head?.ContentType,
        cacheControl: head?.CacheControl,
      })} expected ${JSON.stringify({
        contentType: expected.contentType,
        cacheControl: expected.cacheControl,
      })}`,
    );
  }
  if (
    expected.sourceRevision !== undefined
    && observed.sourceRevision !== expected.sourceRevision
  ) {
    throw new Error(
      `immutable release object source revision mismatch: `
      + `${observed.sourceRevision ?? "missing"} expected ${expected.sourceRevision}`,
    );
  }
  return observed;
}

export function validateReleaseDocuments({
  latest,
  manifest,
  tag,
  baseUrl,
  sourceRevision,
}) {
  const version = tag.replace(/^v/, "");
  const releasePrefix = `${baseUrl}/releases/${tag}/`;
  if (latest?.version !== version) {
    throw new Error(`updater latest version is ${latest?.version}, expected ${version}`);
  }
  if (
    !/^[0-9a-f]{40}$/.test(sourceRevision || "")
    || latest?.source_revision !== sourceRevision
    || manifest?.source_revision !== sourceRevision
  ) {
    throw new Error("release documents do not identify the exact source revision");
  }
  const platforms = Object.values(latest.platforms ?? {});
  if (
    platforms.length !== 4
    || platforms.some((entry) => !entry.signature || !entry.url.startsWith(releasePrefix))
  ) {
    throw new Error("updater manifest must contain four signed immutable platform artifacts");
  }
  if (manifest?.version !== version || manifest?.tag_name !== tag) {
    throw new Error("download manifest version does not match the release tag");
  }
  const assets = Array.isArray(manifest.assets) ? manifest.assets : [];
  const byName = new Map(assets.map((asset) => [asset.name, asset]));
  if (byName.size !== EXPECTED_ALIASES.length) {
    throw new Error(`download manifest must contain ${EXPECTED_ALIASES.length} unique assets`);
  }
  for (const alias of EXPECTED_ALIASES) {
    const asset = byName.get(alias);
    if (
      !asset
      || asset.browser_download_url !== `${releasePrefix}${alias}`
      || !/^sha256:[0-9a-f]{64}$/.test(asset.digest ?? "")
      || !Number.isSafeInteger(asset.size)
      || asset.size <= 0
    ) {
      throw new Error(`download manifest has an invalid ${alias} record`);
    }
  }
  return { version, byName };
}

export function validateRenderedDownloadLinks({ hrefs, baseUrl }) {
  const expected = new Set(EXPECTED_ALIASES.map((alias) => `${baseUrl}/latest/${alias}`));
  const observed = new Set(hrefs.filter((href) => href.startsWith(`${baseUrl}/latest/`)));
  const missing = [...expected].filter((href) => !observed.has(href));
  const unexpected = [...observed].filter((href) => !expected.has(href));
  if (missing.length || unexpected.length) {
    throw new Error(
      `rendered download links differ from the release channel; missing=${missing.join(",")} unexpected=${unexpected.join(",")}`,
    );
  }
  return [...observed].sort();
}

export function validateChannelAdvance(currentDocument, incomingVersion, incomingRevision) {
  const parts = (version) => {
    if (!/^\d+(?:\.\d+)+$/.test(version || "")) {
      throw new Error(`Invalid release version: ${version}`);
    }
    return version.split(".").map(Number);
  };
  if (!/^[0-9a-f]{40}$/.test(incomingRevision || "")) {
    throw new Error("Incoming channel revision is not one exact Git commit");
  }
  const currentVersion = currentDocument?.version;
  const currentRevision = currentDocument?.source_revision;
  if (
    currentRevision !== undefined
    && currentRevision !== null
    && !/^[0-9a-f]{40}$/.test(currentRevision)
  ) {
    throw new Error("Current authoritative channel pointer has a malformed source revision");
  }
  const currentParts = parts(currentVersion);
  const incomingParts = parts(incomingVersion);
  const width = Math.max(currentParts.length, incomingParts.length);
  let direction = 0;
  for (let index = 0; index < width; index += 1) {
    const delta = (currentParts[index] || 0) - (incomingParts[index] || 0);
    if (delta !== 0) {
      direction = Math.sign(delta);
      break;
    }
  }
  if (direction > 0) {
    throw new Error(
      `Refusing to move latest backward from ${currentVersion} to ${incomingVersion}`,
    );
  }
  if (direction === 0 && currentRevision !== incomingRevision) {
    throw new Error(
      `Refusing to replace ${currentVersion} source `
      + `${currentRevision ?? "missing"} with ${incomingRevision}`,
    );
  }
  return {
    currentVersion,
    currentRevision: currentRevision ?? null,
    incomingVersion,
    incomingRevision,
    legacyRevisionMigration: direction < 0 && currentRevision == null,
  };
}

export function validatePublicReleaseJourneyReceipt(receipt, expectedRevision) {
  if (
    receipt?.schema_version !== 2
    || !/^[0-9a-f]{40}$/.test(expectedRevision || "")
    || receipt?.benchmark !== "clark_code_public_release_journey"
    || receipt?.status !== "passed"
    || receipt?.source_revision !== expectedRevision
    || !/^v\d+\.\d+\.\d+$/.test(receipt?.tag || "")
    || receipt?.version !== receipt.tag.slice(1)
    || receipt?.base_url !== "https://downloads.clarkchat.com/desktop"
    || !Array.isArray(receipt?.rendered)
    || receipt.rendered.length !== 2
    || !Array.isArray(receipt?.artifacts)
    || receipt.artifacts.length !== EXPECTED_ALIASES.length
  ) {
    throw new Error("public release journey receipt is missing, stale, or malformed");
  }
  const aliases = new Set();
  for (const artifact of receipt.artifacts) {
    const expectedUrl =
      `${receipt.base_url}/releases/${receipt.tag}/${artifact.alias}`;
    if (
      !EXPECTED_ALIASES.includes(artifact.alias)
      || aliases.has(artifact.alias)
      || artifact.immutable_url !== expectedUrl
      || artifact.immutable?.version !== receipt.tag
      || artifact.publicAlias?.version !== receipt.tag
      || artifact.immutable?.sourceRevision !== receipt.source_revision
      || artifact.publicAlias?.sourceRevision !== receipt.source_revision
      || artifact.immutable?.sha256 !== artifact.publicAlias?.sha256
      || artifact.immutable?.size !== artifact.publicAlias?.size
      || artifact.immutable?.contentSha256 !== artifact.immutable?.sha256
      || artifact.publicAlias?.contentSha256 !== artifact.publicAlias?.sha256
      || artifact.immutable?.contentSize !== artifact.immutable?.size
      || artifact.publicAlias?.contentSize !== artifact.publicAlias?.size
      || !/^[0-9a-f]{64}$/.test(artifact.immutable?.sha256 || "")
      || !Number.isSafeInteger(artifact.immutable?.size)
      || artifact.immutable.size <= 0
    ) {
      throw new Error(`public release journey has an invalid ${artifact.alias} identity`);
    }
    aliases.add(artifact.alias);
  }
  if (aliases.size !== EXPECTED_ALIASES.length) {
    throw new Error("public release journey does not contain every installer");
  }
  return receipt;
}

async function fetchJson(url) {
  const response = await fetch(url, {
    headers: { "cache-control": "no-cache, no-store, max-age=0", pragma: "no-cache" },
  });
  if (!response.ok) throw new Error(`${url} returned HTTP ${response.status}`);
  return response.json();
}

export async function verifyArtifact(url, {
  tag,
  sha256,
  size,
  sourceRevision,
}) {
  const response = await fetch(url, {
    method: "GET",
    headers: { "cache-control": "no-cache, no-store, max-age=0", pragma: "no-cache" },
    signal: AbortSignal.timeout(600_000),
  });
  if (!response.ok) throw new Error(`${url} returned HTTP ${response.status}`);
  if (!response.body) throw new Error(`${url} returned no response body`);
  const observed = {
    version: response.headers.get("x-amz-meta-clark-version"),
    sha256: response.headers.get("x-amz-meta-sha256"),
    size: Number(response.headers.get("content-length")),
    sourceRevision: response.headers.get("x-amz-meta-source-revision"),
  };
  validateArtifactIdentity(
    observed,
    { version: tag, sha256, size },
    url,
  );
  if (observed.sourceRevision !== sourceRevision) {
    throw new Error(
      `${url} source revision mismatch: `
      + `${observed.sourceRevision ?? "missing"} expected ${sourceRevision}`,
    );
  }
  const hash = createHash("sha256");
  let contentSize = 0;
  for await (const chunk of response.body) {
    hash.update(chunk);
    contentSize += chunk.byteLength;
  }
  const contentSha256 = hash.digest("hex");
  if (contentSha256 !== sha256 || contentSize !== size) {
    throw new Error(
      `${url} content identity mismatch: `
      + `${JSON.stringify({ contentSha256, contentSize })} `
      + `expected ${JSON.stringify({ sha256, size })}`,
    );
  }
  return { ...observed, contentSha256, contentSize };
}

export async function runPublicReleaseJourney({
  tag,
  baseUrl,
  siteUrl,
  sourceRevision,
  outputDir,
}) {
  if (!/^[0-9a-f]{40}$/.test(sourceRevision || "")) {
    throw new Error("public release journey requires one clean source revision");
  }
  const normalizedBase = normalizeBase(baseUrl);
  const normalizedSite = new URL(siteUrl).href;
  const [latest, manifest] = await Promise.all([
    fetchJson(`${normalizedBase}/latest/latest.json`),
    fetchJson(`${normalizedBase}/latest/manifest.json`),
  ]);
  const { version, byName } = validateReleaseDocuments({
    latest,
    manifest,
    tag,
    baseUrl: normalizedBase,
    sourceRevision,
  });

  const { chromium } = await import("playwright");
  const browser = await chromium.launch({ headless: true });
  const pages = [normalizedSite, new URL("/clark-code", normalizedSite).href];
  const rendered = [];
  try {
    const page = await browser.newPage();
    for (const url of pages) {
      await page.goto(url, { waitUntil: "domcontentloaded" });
      await page.locator(`a[href^="${normalizedBase}/latest/"]`).first().waitFor({
        state: "visible",
        timeout: 20_000,
      });
      const hrefs = await page.locator("a[href]").evaluateAll((links) =>
        links.map((link) => link.href),
      );
      rendered.push({
        url,
        links: validateRenderedDownloadLinks({ hrefs, baseUrl: normalizedBase }),
      });
    }
  } finally {
    await browser.close();
  }

  const artifacts = [];
  for (const alias of EXPECTED_ALIASES) {
    const asset = byName.get(alias);
    const identity = {
      tag,
      sha256: asset.digest.slice("sha256:".length),
      size: asset.size,
      sourceRevision,
    };
    const immutable = await verifyArtifact(asset.browser_download_url, identity);
    const publicAlias = await verifyArtifact(`${normalizedBase}/latest/${alias}`, identity);
    artifacts.push({ alias, immutable_url: asset.browser_download_url, immutable, publicAlias });
  }

  const receipt = {
    schema_version: 2,
    benchmark: "clark_code_public_release_journey",
    status: "passed",
    generated_at: new Date().toISOString(),
    source_revision: sourceRevision,
    tag,
    version,
    site_url: normalizedSite,
    base_url: normalizedBase,
    rendered,
    artifacts,
  };
  if (outputDir) {
    mkdirSync(outputDir, { recursive: true, mode: 0o700 });
    writeFileSync(
      path.join(outputDir, "receipt.json"),
      `${JSON.stringify(receipt, null, 2)}\n`,
      { mode: 0o600 },
    );
  }
  return receipt;
}

async function runCli() {
  const args = process.argv.slice(2);
  const tag = valueArg(args, "--tag");
  if (!/^v\d+\.\d+\.\d+$/.test(tag)) throw new Error("--tag must be a stable vX.Y.Z tag");
  const output = valueArg(args, "--out");
  const receipt = await runPublicReleaseJourney({
    tag,
    baseUrl: valueArg(args, "--base-url"),
    siteUrl: valueArg(args, "--site-url"),
    sourceRevision: valueArg(args, "--source-revision"),
    outputDir: path.resolve(output),
  });
  console.log(JSON.stringify({ status: receipt.status, version: receipt.version }));
  console.log(`RECEIPT=${path.resolve(output, "receipt.json")}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await runCli();
}
