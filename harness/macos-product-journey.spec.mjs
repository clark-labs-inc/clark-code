import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  MACOS_QA_DATA_STORE_BYTES,
  MACOS_QA_DATA_STORE_UUID,
  MACOS_QA_MODEL,
  MACOS_QA_WINDOW_TITLE,
  assertTargetOutputPath,
  buildStoreHelper,
  fingerprintStateRoots,
  newDesktopKeyIds,
  parseHelperResult,
  redact,
  repoDir,
  runStoreHelper,
  writeBootstrap,
} from "./macos-qa-profile.mjs";

test("macOS QA Tauri config pins the dedicated persistent data store", () => {
  const config = JSON.parse(
    readFileSync(path.join(repoDir, "src-tauri", "tauri.qa.macos.conf.json"), "utf8"),
  );
  assert.equal(config.app.windows[0].title, MACOS_QA_WINDOW_TITLE);
  assert.deepEqual(
    config.app.windows[0].dataStoreIdentifier,
    MACOS_QA_DATA_STORE_BYTES,
  );
  const bytes = Buffer.from(MACOS_QA_DATA_STORE_BYTES);
  const hex = bytes.toString("hex");
  assert.equal(
    `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`,
    MACOS_QA_DATA_STORE_UUID,
  );
});

test("signed launcher has isolated build/launch modes and no personal label", () => {
  const launcher = readFileSync(path.join(repoDir, "script", "build_and_run.sh"), "utf8");
  assert.match(launcher, /--qa-build/);
  assert.match(launcher, /--qa-launch/);
  assert.match(launcher, /CFFIXED_USER_HOME/);
  assert.match(launcher, /CLARK_COMPUTER_USE_DATA_DIR/);
  assert.match(launcher, /tauri\.qa\.macos\.conf\.json/);
  assert.doesNotMatch(launcher, /@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/);
});

test("output path must remain under the repository target directory", () => {
  assert.equal(
    assertTargetOutputPath(path.join(repoDir, "target", "macos-qa-example")),
    path.join(repoDir, "target", "macos-qa-example"),
  );
  assert.throws(
    () => assertTargetOutputPath(path.join(os.tmpdir(), "macos-qa-example")),
    /inside repository target/,
  );
  assert.throws(
    () => assertTargetOutputPath(path.join(repoDir, "target")),
    /inside repository target/,
  );
});

test("receipt redaction removes provider keys, JWTs, and email addresses", () => {
  const source = [
    "ck_live_secretvalue",
    "sk-secretvalue0123456789",
    "eyJhbGciOiJub25lIn0.eyJzdWIiOiJxYSJ9.signature",
    "qa@clarkslabs.com",
  ].join(" ");
  const result = redact(source);
  assert.doesNotMatch(result, /secretvalue/);
  assert.doesNotMatch(result, /eyJ/);
  assert.doesNotMatch(result, /@clarkslabs\.com/);
});

test("disposable key selection ignores existing and non-desktop keys", () => {
  const before = [
    { id: "00000000-0000-0000-0000-000000000001", purpose: "clark_code_desktop" },
  ];
  const after = [
    ...before,
    { id: "00000000-0000-0000-0000-000000000002", purpose: "general" },
    { id: "00000000-0000-0000-0000-000000000003", purpose: "clark_code_desktop" },
  ];
  assert.deepEqual(
    newDesktopKeyIds(before, after),
    ["00000000-0000-0000-0000-000000000003"],
  );
});

test("personal-state fingerprint is stable and detects content mutation", () => {
  const root = mkdtempSync(path.join(repoDir, "target", "macos-state-hash-"));
  try {
    const fixture = path.join(root, "state");
    mkdirSync(fixture, { mode: 0o700 });
    const file = path.join(fixture, "value");
    writeFileSync(file, "first", { mode: 0o600 });
    const roots = [{ label: "fixture", path: fixture }];
    const first = fingerprintStateRoots(roots);
    const second = fingerprintStateRoots(roots);
    assert.equal(first.digest, second.digest);
    writeFileSync(file, "second", { mode: 0o600 });
    const third = fingerprintStateRoots(roots);
    assert.notEqual(first.digest, third.digest);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("helper result parser selects the final safe JSON payload", () => {
  const result = parseHelperResult({
    stdout: "runtime note\n{\"status\":\"passed\",\"credential_recorded\":false}\n",
    stderr: "",
    exit_code: 0,
    duration_ms: 12,
  });
  assert.equal(result.status, "passed");
  assert.equal(result.credential_recorded, false);
  assert.equal(result.helper_exit_code, 0);
});

test(
  "custom Clark origin persists only inside the disposable QA home",
  { skip: process.platform !== "darwin" },
  () => {
    const root = mkdtempSync(path.join(repoDir, "target", "macos-store-smoke-"));
    const marker = randomUUID();
    const workspaceRoot = path.join(
      repoDir,
      "target",
      "macos-qa-workspaces",
      marker,
    );
    const workspace = path.join(workspaceRoot, "ClarkCodeQA");
    const qaHome = path.join(root, "home");
    try {
      mkdirSync(path.join(qaHome, "tmp"), { recursive: true, mode: 0o700 });
      mkdirSync(workspace, { recursive: true, mode: 0o700 });
      const helper = buildStoreHelper(root);
      const bootstrap = path.join(root, "bootstrap.json");
      writeBootstrap(bootstrap, {
        auth_session: {
          user: {
            id: "deterministic-qa-account",
            name: "Clark QA",
            email: "deterministic@clarkslabs.com",
            method: "local",
          },
          clark: {
            endpoint: "wss://example.invalid/ws",
            token: "eyJhbGciOiJub25lIn0.eyJzdWIiOiJxYSJ9.signature",
          },
        },
        cwd: workspace,
        model: MACOS_QA_MODEL,
        marker,
      });
      const seeded = runStoreHelper({
        helperPath: helper.executable_path,
        qaHome,
        operation: "seed",
        args: [bootstrap],
      });
      assert.equal(seeded.status, "passed");
      assert.equal(seeded.credential_recorded, false);
      assert.equal(
        readFileSync(bootstrap, "utf8").includes("deterministic@clarkslabs.com"),
        true,
      );
      assert.equal(
        existsSync(
          path.join(
            qaHome,
            "Library",
            "WebKit",
            "com.clark.desktop.dev",
            "WebsiteDataStore",
          ),
        ),
        true,
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
      rmSync(workspaceRoot, { recursive: true, force: true });
    }
  },
);

test("direct WebKit data-store removal is absent because used-store removal crashes", () => {
  const source = readFileSync(
    path.join(repoDir, "harness", "macos-webkit-data-store.swift"),
    "utf8",
  );
  assert.doesNotMatch(source, /WKWebsiteDataStore\.remove/);
});
