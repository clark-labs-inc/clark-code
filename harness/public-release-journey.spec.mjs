import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import test from "node:test";

import {
  createLegacyUpdaterDocument,
  createPlatformDownloadManifest,
  validatePublicReleaseJourneyReceipt,
  validateChannelAdvance,
  createPlatformUpdaterDocuments,
  PLATFORM_RELEASES,
  validateS3HeadObject,
  validateReleaseDocuments,
  validatePlatformDownloadManifest,
  validatePlatformReleaseAssets,
  validateLegacyUpdaterDocument,
  validatePlatformUpdaterDocument,
  validateRenderedDownloadLinks,
  verifyArtifact,
} from "./public-release-journey.mjs";
import {
  candidateIdentityFromHeaders,
  validateReleaseCandidateDownload,
  validateWindowsReleaseBuildReceipt,
} from "./download-release-candidate.mjs";
import {
  buildWindowsUpdateCandidate,
  validateWindowsUpdateCandidateReceipt,
} from "./windows-update-candidate.mjs";

const baseUrl = "https://downloads.example.test/desktop";
const tag = "v1.2.3";
const sourceRevision = "f".repeat(40);
const aliases = [
  "ClarkCode.dmg",
  "ClarkCode_x64-setup.exe",
  "ClarkCode_amd64.AppImage",
  "ClarkCode_amd64.deb",
  "ClarkCode_x86_64.rpm",
];

test("candidate release is parallel, supply-chain bound, and platform independent", () => {
  const workflow = readFileSync(
    new URL("../.github/workflows/release.yml", import.meta.url),
    "utf8",
  );
  const signingConfig = readFileSync(
    new URL("../src-tauri/tauri.windows-signing.conf.json", import.meta.url),
    "utf8",
  );
  const signingScript = readFileSync(
    new URL("../src-tauri/sign-windows-artifact.ps1", import.meta.url),
    "utf8",
  );
  const preparationScript = readFileSync(
    new URL("../scripts/prepare-windows-artifact-signing.ps1", import.meta.url),
    "utf8",
  );

  const pinnedActions = [...workflow.matchAll(/uses:\s+\S+@([0-9a-f]+)/g)];
  assert.ok(pinnedActions.length > 0);
  assert.deepEqual(
    pinnedActions.filter((match) => match[1].length !== 40),
    [],
    "every release action must use a full 40-character commit SHA",
  );
  assert.doesNotMatch(workflow, /WINDOWS_CERTIFICATE(?:_PASSWORD|_THUMBPRINT)?/);
  assert.match(
    workflow,
    /workflow_dispatch:[\s\S]*?version:[\s\S]*?run_paid_benchmark:/,
  );
  assert.doesNotMatch(workflow, /\n\s+push:\s*\n\s+tags:/);
  assert.match(
    workflow,
    /concurrency:[\s\S]*?group: clark-desktop-release-candidate[\s\S]*?cancel-in-progress: false/,
  );
  assert.match(
    workflow,
    /release_source_prerequisites:[\s\S]*?runs-on: ubuntu-latest[\s\S]*?environment: release/,
  );
  assert.match(
    workflow,
    /windows_release_prerequisites:[\s\S]*?runs-on: windows-latest[\s\S]*?vars\.CLARK_WINDOWS_RELEASE_MODE == 'signed'/,
  );
  assert.match(
    workflow,
    /\n  publish:[\s\S]*?runs-on: \[self-hosted, macOS, ARM64, clark-utm-qa\][\s\S]*?environment: release/,
  );
  assert.match(
    workflow,
    /\n  exec-server:[\s\S]*?runs-on: macos-latest[\s\S]*?environment: release/,
  );
  assert.match(
    workflow,
    /fetch-depth: 0[\s\S]*?tauri\.conf\.json[\s\S]*?declared_version[\s\S]*?git rev-parse origin\/main\)" != "\$GITHUB_SHA"/,
  );
  assert.match(
    workflow,
    /\n  build:[\s\S]*?runs-on: \$\{\{ matrix\.platform \}\}[\s\S]*?environment: release/,
  );
  assert.match(workflow, /uses: azure\/login@[0-9a-f]{40}/g);
  assert.match(
    workflow,
    /windows_release_prerequisites:[\s\S]*?sign-windows-artifact\.ps1[\s\S]*?SignerCertificate\.Subject/,
  );
  assert.match(
    workflow,
    /release_source_prerequisites:[\s\S]*?aws-actions\/configure-aws-credentials@[0-9a-f]{40}[\s\S]*?aws sts get-caller-identity/,
  );
  assert.match(
    workflow,
    /windows_utm_runner_preflight:[\s\S]*?needs: \[windows_release_prerequisites\]/,
  );
  assert.match(
    workflow,
    /pre_release_benchmarks:[\s\S]*?needs: \[release_source_prerequisites\]/,
  );
  assert.match(workflow, /permissions:[\s\S]*?actions: read/);
  assert.match(
    workflow,
    /release_source_ci:[\s\S]*?needs: \[release_source_prerequisites\][\s\S]*?--workflow ci\.yml[\s\S]*?--commit "\$GITHUB_SHA"[\s\S]*?run_conclusion[\s\S]*?success/,
  );
  assert.match(
    workflow,
    /\n  build:\s*\n\s+needs: \[release_source_prerequisites\]/,
  );
  assert.match(
    workflow,
    /\n  exec-server:\s*\n\s+needs: \[release_source_prerequisites\]/,
  );
  assert.match(
    workflow,
    /publish_independent_platforms:[\s\S]*?needs: \[build, exec-server, pre_release_benchmarks, release_source_ci\]/,
  );
  assert.match(
    workflow,
    /publish_independent_platforms:[\s\S]*?Publish independently verified platform downloads[\s\S]*?desktop\/latest\/updater\//,
  );
  assert.match(
    workflow,
    /publish_independent_platforms:[\s\S]*?desktop\/latest\/latest\.json[\s\S]*?aws s3 cp "\$release_dir\/latest\.json"/,
  );
  assert.match(
    workflow,
    /publish_independent_platforms:[\s\S]*?desktop\/latest\/manifest\.json[\s\S]*?aws s3 cp "\$release_dir\/manifest\.json"/,
  );
  assert.ok(
    workflow.includes('}, null, 2) + "\\n",'),
    "publication receipts must end with an actual newline",
  );
  assert.ok(
    !workflow.includes('}, null, 2) + "\\\\n",'),
    "publication receipts must not append a literal backslash-n",
  );
  assert.match(
    workflow,
    /Build installers \(non-macOS\)[\s\S]*?if: runner\.os != 'macOS'[\s\S]*?uses: tauri-apps\/tauri-action@[0-9a-f]{40}[\s\S]*?with:\s*\n\s*args:/,
  );
  assert.match(
    workflow,
    /for secret_name in APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY; do[\s\S]*?::error::\$secret_name is required for every macOS release[\s\S]*?security import/,
  );
  assert.match(
    workflow,
    /certificate_path="\$certificate_dir\/cert\.p12"[\s\S]*?security import "\$certificate_path"/,
  );
  assert.match(
    workflow,
    /Build installers \(signed macOS\)[\s\S]*?if: runner\.os == 'macOS'[\s\S]*?APPLE_CERTIFICATE: \$\{\{ secrets\.APPLE_CERTIFICATE \}\}/,
  );
  assert.match(
    workflow,
    /Verify packaged macOS runtime layout[\s\S]*?Every macOS release has a Developer ID-signed Computer Use service\.[\s\S]*?"\$helper" --self-test/,
  );
  assert.doesNotMatch(workflow, /building macOS release unsigned|unsigned or non-macOS|signing_enabled != 'true'/);
  assert.doesNotMatch(workflow, /\b(?:tagName|releaseName|releaseBody|releaseDraft):/);
  assert.doesNotMatch(workflow, /\bgh release\b/);
  assert.match(
    workflow,
    /Generate platform SPDX SBOM[\s\S]*?anchore\/sbom-action@[0-9a-f]{40}[\s\S]*?actions\/attest@[0-9a-f]{40}/,
  );
  assert.match(
    workflow,
    /finalize_release_tag:[\s\S]*?Reverify public generation and create the stable tag[\s\S]*?ClarkCode_x64-setup\.exe[\s\S]*?windows-x86_64[\s\S]*?git tag -a "\$CLARK_RELEASE_TAG"[\s\S]*?git push origin/,
  );
  assert.match(
    workflow,
    /windows-release-build-receipt[\s\S]*?--build-receipt target\/windows-release-build\/receipt\.json/,
  );
  assert.match(
    workflow,
    /_channel-rollback\/\$\{GITHUB_RUN_ID\}-\$\{GITHUB_RUN_ATTEMPT\}[\s\S]*?backup-ready/,
  );
  assert.match(
    workflow,
    /aws s3api get-object[\s\S]*?desktop\/latest\/latest\.json[\s\S]*?validateChannelAdvance/,
  );
  assert.match(
    workflow,
    /Rollback snapshot \$\{key\} changed \$\{field\}[\s\S]*?"ETag"[\s\S]*?ChecksumSHA256/,
  );
  assert.match(
    workflow,
    /Restore the prior public channel after a failed release[\s\S]*?failure\(\) \|\| cancelled\(\)[\s\S]*?commit-receipt\.json[\s\S]*?metadata-directive COPY/,
  );
  assert.match(
    workflow,
    /Restored \$\{key\} changed \$\{field\}[\s\S]*?clark_code_public_channel_rollback[\s\S]*?objects_restored: 11/,
  );
  assert.match(
    workflow,
    /Upload public channel transaction receipt[\s\S]*?if: always\(\)[\s\S]*?public-channel-transaction/,
  );
  assert.match(
    workflow,
    /aws s3 cp "\$manifest"[\s\S]*?aws s3 cp "\$latest"/,
  );
  assert.match(
    workflow,
    /windows_release_vm_cleanup:[\s\S]*?if: \$\{\{ always\(\) && needs\.windows_packaged_journey\.result != 'skipped' \}\}[\s\S]*?--action delete-clone/,
  );
  assert.match(workflow, /src-tauri\/tauri\.windows-signing\.conf\.json/g);
  const desktopConfig = JSON.parse(readFileSync(
    new URL("../src-tauri/tauri.conf.json", import.meta.url),
    "utf8",
  ));
  assert.deepEqual(desktopConfig.plugins.updater.endpoints, [
    "https://downloads.clarkchat.com/desktop/latest/updater/{{target}}-{{arch}}.json",
  ]);
  const parsedSigningConfig = JSON.parse(signingConfig);
  assert.deepEqual(
    parsedSigningConfig.bundle.windows.signCommand,
    {
      cmd: "powershell.exe",
      args: [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        "sign-windows-artifact.ps1",
        "-FilePath",
        "%1",
      ],
    },
  );
  assert.match(signingScript, /http:\/\/timestamp\.acs\.microsoft\.com/);
  assert.match(signingScript, /verify \/v \/pa \/all/);
  assert.match(preparationScript, /ArtifactSigningClientTools\.msi/);
  assert.doesNotMatch(preparationScript, /"AzureCliCredential"/);
});

test("authoritative channel advancement permits only monotonic exact generations", () => {
  assert.equal(
    validateChannelAdvance(
      { version: "1.2.3", source_revision: sourceRevision },
      "1.2.3",
      sourceRevision,
    ).legacyRevisionMigration,
    false,
  );
  assert.equal(
    validateChannelAdvance(
      { version: "1.2.3" },
      "1.2.4",
      "e".repeat(40),
    ).legacyRevisionMigration,
    true,
  );
  assert.throws(
    () => validateChannelAdvance(
      { version: "1.2.4", source_revision: sourceRevision },
      "1.2.3",
      "e".repeat(40),
    ),
    /move latest backward/,
  );
  assert.throws(
    () => validateChannelAdvance(
      { version: "1.2.3" },
      "1.2.3",
      sourceRevision,
    ),
    /source missing/,
  );
  assert.throws(
    () => validateChannelAdvance(
      { version: "1.2.3", source_revision: "not-a-revision" },
      "1.2.4",
      sourceRevision,
    ),
    /malformed source revision/,
  );
});

function documents() {
  return {
    latest: {
      version: "1.2.3",
      source_revision: sourceRevision,
      platforms: Object.fromEntries(
        ["darwin-aarch64", "darwin-x86_64", "windows-x86_64", "linux-x86_64"]
          .map((platform) => [
            platform,
            {
              signature: "signed",
              url: `${baseUrl}/releases/${tag}/${platform}`,
            },
          ]),
      ),
    },
    manifest: {
      version: "1.2.3",
      tag_name: tag,
      source_revision: sourceRevision,
      assets: aliases.map((name) => ({
        name,
        browser_download_url: `${baseUrl}/releases/${tag}/${name}`,
        digest: `sha256:${"a".repeat(64)}`,
        size: 42,
      })),
    },
  };
}

test("independent platform release documents preserve exact platform boundaries", () => {
  const independentAliases = [
    ...PLATFORM_RELEASES.macos.aliases,
    ...PLATFORM_RELEASES.linux.aliases,
  ];
  const assets = independentAliases.map((name) => ({
    name,
    browser_download_url: `${baseUrl}/releases/${tag}/${name}`,
    digest: `sha256:${"b".repeat(64)}`,
    size: 42,
  }));
  const validated = validatePlatformReleaseAssets({
    assets,
    tag,
    baseUrl,
    sourceRevision,
    platforms: ["macos", "linux"],
  });
  assert.deepEqual(validated.aliases, independentAliases);
  assert.throws(
    () => validatePlatformReleaseAssets({
      assets: assets.slice(1),
      tag,
      baseUrl,
      sourceRevision,
      platforms: ["macos", "linux"],
    }),
    /unexpected set of website installers/,
  );
  const manifest = createPlatformDownloadManifest({
    assets,
    tag,
    baseUrl,
    sourceRevision,
    platforms: ["macos", "linux"],
    publishedAt: "2026-07-29T00:00:00.000Z",
  });
  assert.deepEqual(
    validatePlatformDownloadManifest({
      manifest,
      tag,
      baseUrl,
      sourceRevision,
      platforms: ["macos", "linux"],
    }).aliases,
    independentAliases,
  );
  const staleManifest = structuredClone(manifest);
  staleManifest.version = "1.2.2";
  assert.throws(
    () => validatePlatformDownloadManifest({
      manifest: staleManifest,
      tag,
      baseUrl,
      sourceRevision,
      platforms: ["macos", "linux"],
    }),
    /complete platform release/,
  );

  const updater = createPlatformUpdaterDocuments({
    updaterFragments: [{
      platforms: {
        "darwin-aarch64": {
          signature: "mac-signature",
          url: `${baseUrl}/releases/${tag}/ClarkCode.app.tar.gz`,
        },
        "darwin-x86_64": {
          signature: "mac-signature",
          url: `${baseUrl}/releases/${tag}/ClarkCode.app.tar.gz`,
        },
        "linux-x86_64": {
          signature: "linux-signature",
          url: `${baseUrl}/releases/${tag}/ClarkCode_amd64.AppImage`,
        },
      },
    }],
    tag,
    baseUrl,
    sourceRevision,
    platforms: ["macos", "linux"],
    repository: "clark-labs-inc/clark-desktop",
    publishedAt: "2026-07-28T00:00:00.000Z",
  });
  assert.deepEqual(Object.keys(updater).sort(), [
    "darwin-aarch64",
    "darwin-x86_64",
    "linux-x86_64",
  ]);
  for (const [target, document] of Object.entries(updater)) {
    assert.equal(
      validatePlatformUpdaterDocument({
        document,
        target,
        tag,
        baseUrl,
        sourceRevision,
      }),
      document.platforms[target],
    );
  }
  const legacy = createLegacyUpdaterDocument({
    updaterDocuments: updater,
    tag,
    baseUrl,
    sourceRevision,
    platforms: ["macos", "linux"],
  });
  assert.deepEqual(Object.keys(legacy.platforms), [
    "darwin-aarch64",
    "darwin-x86_64",
    "linux-x86_64",
  ]);
  assert.equal(
    validateLegacyUpdaterDocument({
      document: legacy,
      platformDocuments: updater,
      tag,
      baseUrl,
      sourceRevision,
      platforms: ["macos", "linux"],
    }),
    legacy,
  );
  const staleLegacy = structuredClone(legacy);
  staleLegacy.version = "1.2.2";
  assert.throws(
    () => validateLegacyUpdaterDocument({
      document: staleLegacy,
      platformDocuments: updater,
      tag,
      baseUrl,
      sourceRevision,
      platforms: ["macos", "linux"],
    }),
    /complete platform release/,
  );
});

test("release documents require exact immutable platform and installer identities", () => {
  const value = documents();
  assert.equal(
    validateReleaseDocuments({
      ...value,
      tag,
      baseUrl,
      sourceRevision,
    }).version,
    "1.2.3",
  );
  value.manifest.assets[1].digest = `sha256:${"b".repeat(63)}`;
  assert.throws(
    () => validateReleaseDocuments({
      ...value,
      tag,
      baseUrl,
      sourceRevision,
    }),
    /invalid ClarkCode_x64-setup.exe/,
  );
});

test("rendered site links must expose every exact public installer alias", () => {
  const hrefs = aliases.map((name) => `${baseUrl}/latest/${name}`);
  assert.equal(validateRenderedDownloadLinks({ hrefs, baseUrl }).length, aliases.length);
  hrefs[1] = `${baseUrl}/releases/v1.1.0/ClarkCode_x64-setup.exe`;
  assert.throws(
    () => validateRenderedDownloadLinks({ hrefs, baseUrl }),
    /missing=.*ClarkCode_x64-setup.exe/,
  );
});

test("immutable S3 identities are reusable only when bytes and metadata are exact", () => {
  const expected = {
    tag,
    sha256: "a".repeat(64),
    size: 42,
    contentType: "application/vnd.microsoft.portable-executable",
    cacheControl: "public, max-age=31536000, immutable",
    sourceRevision,
  };
  const head = {
    Metadata: {
      "clark-version": tag,
      sha256: expected.sha256,
      "source-revision": sourceRevision,
    },
    ContentLength: expected.size,
    ContentType: expected.contentType,
    CacheControl: expected.cacheControl,
  };
  assert.equal(validateS3HeadObject(head, expected).sha256, expected.sha256);
  assert.throws(
    () => validateS3HeadObject(
      { ...head, Metadata: { ...head.Metadata, sha256: "b".repeat(64) } },
      expected,
    ),
    /identity mismatch/,
  );
  assert.throws(
    () => validateS3HeadObject({ ...head, CacheControl: "no-store" }, expected),
    /metadata mismatch/,
  );
  assert.throws(
    () => validateS3HeadObject(
      { ...head, Metadata: { ...head.Metadata, "source-revision": "b".repeat(40) } },
      expected,
    ),
    /source revision mismatch/,
  );
});

test("release candidate response headers require an exact immutable identity", () => {
  const headers = new Headers({
    "x-amz-meta-clark-version": tag,
    "x-amz-meta-sha256": "a".repeat(64),
    "x-amz-meta-source-revision": sourceRevision,
    "content-length": "42",
  });
  assert.deepEqual(candidateIdentityFromHeaders(headers), {
    version: tag,
    sha256: "a".repeat(64),
    size: 42,
    sourceRevision,
  });
  headers.set("content-length", "0");
  assert.throws(
    () => candidateIdentityFromHeaders(headers),
    /invalid identity metadata/,
  );
});

test("download receipts bind public candidate bytes to the source revision", () => {
  const sourceRevision = "b".repeat(40);
  const receipt = {
    schema_version: 1,
    receipt_type: "clark_code_release_candidate_download",
    status: "passed",
    source_revision: sourceRevision,
    tag,
    version: tag.slice(1),
    base_url: baseUrl,
    signer_subject: "CN=Clark Labs Inc., O=Clark Labs Inc., C=US",
    signer_thumbprint: "A".repeat(40),
    build_receipt_sha256: "c".repeat(64),
    artifact: {
      asset: "ClarkCode_x64-setup.exe",
      url: `${baseUrl}/releases/${tag}/ClarkCode_x64-setup.exe`,
      source_revision: sourceRevision,
      sha256: "a".repeat(64),
      size: 42,
      file: "ClarkCode_x64-setup.exe",
    },
  };
  assert.equal(validateReleaseCandidateDownload(receipt, sourceRevision), receipt);
  assert.throws(
    () => validateReleaseCandidateDownload(receipt, "c".repeat(40)),
    /missing, stale, or malformed/,
  );
});

test("Windows build receipts bind CDN bytes to source and Clark signer", () => {
  const receipt = {
    schema_version: 1,
    receipt_type: "clark_code_windows_release_build",
    status: "passed",
    source_revision: sourceRevision,
    tag,
    version: tag.slice(1),
    signer_subject: "CN=Clark Labs Inc., O=Clark Labs Inc., C=US",
    signer_thumbprint: "A".repeat(40),
    artifact: {
      asset: "ClarkCode_x64-setup.exe",
      url: `${baseUrl}/releases/${tag}/ClarkCode_x64-setup.exe`,
      sha256: "a".repeat(64),
      size: 42,
    },
  };
  assert.equal(
    validateWindowsReleaseBuildReceipt(receipt, sourceRevision, tag, baseUrl),
    receipt,
  );
  assert.throws(
    () => validateWindowsReleaseBuildReceipt(
      { ...receipt, signer_thumbprint: "not-a-thumbprint" },
      sourceRevision,
      tag,
      baseUrl,
    ),
    /missing, stale, or malformed/,
  );
});

test("Windows candidate updater channel is immutable and source-bound", () => {
  const sourceRevision = "c".repeat(40);
  const result = buildWindowsUpdateCandidate({
    tag,
    baseUrl: "https://downloads.clarkchat.com/desktop",
    sourceRevision,
    publishedAt: "2026-07-27T00:00:00.000Z",
    updaterFragment: {
      platforms: {
        "windows-x86_64": {
          signature: "signed-update-payload".repeat(3),
          url:
            `https://downloads.clarkchat.com/desktop/releases/${tag}/ClarkCode_x64-setup.exe`,
        },
      },
    },
  });
  assert.equal(result.manifest.version, tag.slice(1));
  assert.equal(
    result.seedConfig.plugins.updater.endpoints[0],
    `https://downloads.clarkchat.com/desktop/releases/${tag}/windows-update.json`,
  );
  assert.equal(
    validateWindowsUpdateCandidateReceipt(result.receipt, sourceRevision),
    result.receipt,
  );
  assert.throws(
    () => validateWindowsUpdateCandidateReceipt(result.receipt, "d".repeat(40)),
    /missing, stale, or malformed/,
  );
});

test("public journey receipts require source-bound immutable and alias identities", () => {
  const sourceRevision = "e".repeat(40);
  const artifacts = aliases.map((alias) => ({
    alias,
    immutable_url:
      `https://downloads.clarkchat.com/desktop/releases/${tag}/${alias}`,
    immutable: {
      version: tag,
      sha256: "a".repeat(64),
      size: 42,
      sourceRevision,
      contentSha256: "a".repeat(64),
      contentSize: 42,
    },
    publicAlias: {
      version: tag,
      sha256: "a".repeat(64),
      size: 42,
      sourceRevision,
      contentSha256: "a".repeat(64),
      contentSize: 42,
    },
  }));
  const receipt = {
    schema_version: 2,
    benchmark: "clark_code_public_release_journey",
    status: "passed",
    source_revision: sourceRevision,
    tag,
    version: tag.slice(1),
    base_url: "https://downloads.clarkchat.com/desktop",
    rendered: [{}, {}],
    artifacts,
  };
  assert.equal(
    validatePublicReleaseJourneyReceipt(receipt, sourceRevision),
    receipt,
  );
  artifacts[0].publicAlias.sha256 = "b".repeat(64);
  assert.throws(
    () => validatePublicReleaseJourneyReceipt(receipt, sourceRevision),
    /invalid ClarkCode.dmg identity/,
  );
  artifacts[0].publicAlias.sha256 = "a".repeat(64);
  artifacts[0].publicAlias.contentSha256 = "b".repeat(64);
  assert.throws(
    () => validatePublicReleaseJourneyReceipt(receipt, sourceRevision),
    /invalid ClarkCode.dmg identity/,
  );
});

test("public artifact verification hashes the downloaded bytes", async (t) => {
  const expectedBody = Buffer.from("exact release bytes");
  const expectedSha256 = createHash("sha256").update(expectedBody).digest("hex");
  let responseBody = expectedBody;
  const server = createServer((_request, response) => {
    response.writeHead(200, {
      "content-length": String(responseBody.length),
      "x-amz-meta-clark-version": tag,
      "x-amz-meta-sha256": expectedSha256,
      "x-amz-meta-source-revision": sourceRevision,
    });
    response.end(responseBody);
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  t.after(() => server.close());
  const address = server.address();
  assert.notEqual(address, null);
  assert.equal(typeof address, "object");
  const url = `http://127.0.0.1:${address.port}/artifact`;
  const expected = {
    tag,
    sha256: expectedSha256,
    size: expectedBody.length,
    sourceRevision,
  };

  const verified = await verifyArtifact(url, expected);
  assert.equal(verified.contentSha256, expectedSha256);
  assert.equal(verified.contentSize, expectedBody.length);

  responseBody = Buffer.from("wrong release bytes");
  await assert.rejects(
    () => verifyArtifact(url, expected),
    /content identity mismatch/,
  );
});
