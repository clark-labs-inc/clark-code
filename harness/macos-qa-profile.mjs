import { spawnSync } from "node:child_process";
import { createDecipheriv, createHash } from "node:crypto";
import {
  chmodSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import {
  isOwnerOnlyFile,
  secureOwnerOnlyFile,
} from "./owner-only-file.mjs";
import { nativeCredentialEnvelope } from "./native-credential-envelope.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
export const repoDir = path.resolve(harnessDir, "..");
export const MACOS_QA_DATA_STORE_UUID = "7496b32d-bc0c-4403-b6c0-c650c65f5b8a";
export const MACOS_QA_DATA_STORE_BYTES = [
  116, 150, 179, 45, 188, 12, 68, 3,
  182, 192, 198, 80, 198, 95, 91, 138,
];
export const MACOS_QA_MODEL = "clark-code:free";
export const MACOS_QA_REMOTE_HOST = "nucleus";
export const MACOS_QA_REMOTE_ROOT = "/tmp/clark-code-registry-smoke";
export const MACOS_QA_WINDOW_TITLE = "Clark Code Dev QA";
const CREDENTIAL_MAGIC = Buffer.from("CLKCRD02");
const CREDENTIAL_AAD = Buffer.from("clark-desktop-credentials-v2");
export const MACOS_APP_BUNDLE = path.join(
  repoDir,
  "target",
  "debug",
  "bundle",
  "macos",
  "Clark Code Dev.app",
);

const helperSource = path.join(harnessDir, "macos-webkit-data-store.swift");
const helperRuntimeSource = path.join(harnessDir, "macos-webkit-runner.swift");
const personalRoots = [
  {
    label: "webkit",
    path: path.join(os.homedir(), "Library", "WebKit", "com.clark.desktop.dev"),
  },
  {
    label: "cache",
    path: path.join(os.homedir(), "Library", "Caches", "com.clark.desktop.dev"),
  },
  {
    label: "native_data",
    path: path.join(
      os.homedir(),
      "Library",
      "Application Support",
      "com.clark.desktop.dev",
    ),
  },
  {
    label: "preferences",
    path: path.join(
      os.homedir(),
      "Library",
      "Preferences",
      "com.clark.desktop.dev.plist",
    ),
  },
  {
    label: "computer_use",
    path: path.join(
      os.homedir(),
      "Library",
      "Application Support",
      "Clark Code",
      "Computer Use",
    ),
  },
];

export function run(command, args, options = {}) {
  const started = Date.now();
  const completed = spawnSync(command, args, {
    cwd: options.cwd ?? repoDir,
    encoding: "utf8",
    env: options.env ?? process.env,
    input: options.input,
    timeout: options.timeout_ms ?? 120_000,
    maxBuffer: options.max_buffer ?? 16 * 1024 * 1024,
  });
  return {
    ok: completed.status === 0,
    exit_code: completed.status,
    signal: completed.signal || null,
    stdout: completed.stdout || "",
    stderr: completed.stderr || completed.error?.message || "",
    duration_ms: Date.now() - started,
  };
}

export function redact(value) {
  return String(value)
    .replace(/\bck_(?:live|test)_[A-Za-z0-9._-]+\b/g, "ck_[REDACTED]")
    .replace(/\bsk-[A-Za-z0-9_-]{16,}\b/g, "sk-[REDACTED]")
    .replace(
      /\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b/g,
      "[JWT_REDACTED]",
    )
    .replace(/(authorization\s*[:=]\s*bearer\s+)\S+/gi, "$1[REDACTED]")
    .replace(/\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b/gi, "[EMAIL_REDACTED]");
}

export function assertTargetOutputPath(outputDir) {
  const resolved = path.resolve(outputDir);
  const targetRoot = path.join(repoDir, "target");
  if (resolved === targetRoot || !resolved.startsWith(`${targetRoot}${path.sep}`)) {
    throw new Error("macOS QA output must be a new directory inside repository target");
  }
  return resolved;
}

function writeBundleInfoPlist(bundlePath) {
  const plistPath = path.join(bundlePath, "Contents", "Info.plist");
  writeFileSync(
    plistPath,
    `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleExecutable</key><string>clark-macos-qa-store</string>
  <key>CFBundleIdentifier</key><string>com.clark.desktop.dev</string>
  <key>CFBundleName</key><string>Clark macOS QA Store</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSBackgroundOnly</key><true/>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
</dict>
</plist>
`,
    { mode: 0o600 },
  );
  return plistPath;
}

export function buildStoreHelper(outputDir) {
  const bundlePath = path.join(outputDir, "tools", "Clark macOS QA Store.app");
  const executableDir = path.join(bundlePath, "Contents", "MacOS");
  mkdirSync(executableDir, { recursive: true, mode: 0o700 });
  chmodSync(path.join(outputDir, "tools"), 0o700);
  chmodSync(bundlePath, 0o700);
  chmodSync(path.join(bundlePath, "Contents"), 0o700);
  chmodSync(executableDir, 0o700);
  const plistPath = writeBundleInfoPlist(bundlePath);
  const executablePath = path.join(executableDir, "clark-macos-qa-store");
  const compiled = run(
    "swiftc",
    [
      helperRuntimeSource,
      helperSource,
      "-framework",
      "AppKit",
      "-framework",
      "WebKit",
      "-o",
      executablePath,
    ],
    { timeout_ms: 120_000 },
  );
  if (!compiled.ok) {
    throw new Error(`could not compile macOS QA store helper: ${redact(compiled.stderr)}`);
  }
  chmodSync(executablePath, 0o700);
  const plist = run("plutil", ["-lint", plistPath]);
  if (!plist.ok) {
    throw new Error(`invalid macOS QA helper bundle: ${redact(plist.stderr)}`);
  }
  const signed = run("codesign", ["--force", "--sign", "-", bundlePath]);
  if (!signed.ok) {
    throw new Error(`could not ad-hoc sign macOS QA helper: ${redact(signed.stderr)}`);
  }
  return {
    bundle_path: bundlePath,
    executable_path: executablePath,
    compile_duration_ms: compiled.duration_ms,
  };
}

export function profileEnvironment(qaHome) {
  return {
    ...process.env,
    HOME: qaHome,
    CFFIXED_USER_HOME: qaHome,
    TMPDIR: path.join(qaHome, "tmp"),
    CLARK_COMPUTER_USE_DATA_DIR: path.join(
      qaHome,
      "Library",
      "Application Support",
      "Clark Code",
      "Computer Use",
    ),
  };
}

export function parseHelperResult(completed) {
  const lines = completed.stdout.trim().split(/\r?\n/).filter(Boolean);
  let parsed = null;
  for (let index = lines.length - 1; index >= 0; index -= 1) {
    try {
      parsed = JSON.parse(lines[index]);
      break;
    } catch {
      // Ignore compiler/runtime chatter and keep looking for the helper payload.
    }
  }
  if (!parsed || typeof parsed !== "object") {
    throw new Error(
      `macOS QA helper returned no JSON: ${redact(completed.stderr || completed.stdout)}`,
    );
  }
  return {
    ...parsed,
    helper_exit_code: completed.exit_code,
    duration_ms: completed.duration_ms,
  };
}

export function runStoreHelper({
  helperPath,
  qaHome,
  operation,
  args = [],
  timeoutMs = 60_000,
}) {
  const completed = run(
    helperPath,
    [operation, MACOS_QA_DATA_STORE_UUID, ...args],
    {
      env: profileEnvironment(qaHome),
      timeout_ms: timeoutMs,
    },
  );
  return parseHelperResult(completed);
}

export function writeBootstrap(filePath, payload) {
  writeFileSync(filePath, `${JSON.stringify(payload)}\n`, { mode: 0o600 });
  secureOwnerOnlyFile(filePath);
  if (!isOwnerOnlyFile(filePath)) {
    throw new Error("macOS QA bootstrap is not owner-only");
  }
}

function credentialRoot(qaHome) {
  return path.join(
    qaHome,
    "Library",
    "Application Support",
    "com.clark.desktop.dev",
    "credentials",
  );
}

/** Seed the isolated packaged app through its real encrypted native boundary. */
export function writeNativeCredentialBootstrap(qaHome, retainedAuth) {
  const root = credentialRoot(qaHome);
  mkdirSync(root, { recursive: true, mode: 0o700 });
  chmodSync(root, 0o700);
  const generated = nativeCredentialEnvelope(retainedAuth);
  const key = Buffer.from(generated.key, "base64");
  const envelope = Buffer.from(generated.envelope, "base64");
  const keyPath = path.join(root, "credentials.key");
  const statePath = path.join(root, "credentials.enc");
  writeFileSync(keyPath, key, { mode: 0o600 });
  writeFileSync(
    statePath,
    envelope,
    { mode: 0o600 },
  );
  secureOwnerOnlyFile(keyPath);
  secureOwnerOnlyFile(statePath);
  return {
    root,
    key_path: keyPath,
    state_path: statePath,
  };
}

/** Verify the product retained auth in its authenticated native envelope. */
export function probeNativeCredentialState(qaHome, expectedRetainedAuth) {
  const root = credentialRoot(qaHome);
  const keyPath = path.join(root, "credentials.key");
  const statePath = path.join(root, "credentials.enc");
  const key = readFileSync(keyPath);
  const envelope = readFileSync(statePath);
  if (!envelope.subarray(0, CREDENTIAL_MAGIC.length).equals(CREDENTIAL_MAGIC)) {
    throw new Error("native credential envelope has an invalid header");
  }
  const nonceStart = CREDENTIAL_MAGIC.length;
  const nonce = envelope.subarray(nonceStart, nonceStart + 12);
  const sealed = envelope.subarray(nonceStart + 12);
  const tag = sealed.subarray(sealed.length - 16);
  const ciphertext = sealed.subarray(0, sealed.length - 16);
  const decipher = createDecipheriv("chacha20-poly1305", key, nonce, {
    authTagLength: 16,
  });
  decipher.setAAD(CREDENTIAL_AAD, { plaintextLength: ciphertext.length });
  decipher.setAuthTag(tag);
  const state = JSON.parse(Buffer.concat([
    decipher.update(ciphertext),
    decipher.final(),
  ]).toString("utf8"));
  const retainedAuth = JSON.parse(state.retained_auth || "null");
  const encryptedSource = envelope.toString("utf8");
  const expectedUser = expectedRetainedAuth.descriptor.user;
  const expectedToken = expectedRetainedAuth.clarkToken;
  return {
    status: (
      state.version === 2
      && retainedAuth?.version === 2
      && retainedAuth?.descriptor?.user?.id === expectedUser.id
      && retainedAuth?.authOrigin === expectedRetainedAuth.authOrigin
      && retainedAuth?.clarkToken === expectedToken
      && !encryptedSource.includes(expectedToken)
      && !encryptedSource.includes(expectedUser.email)
    ) ? "passed" : "failed",
    operation: "probe_native_credentials",
    encrypted: true,
    auth_retained: retainedAuth?.descriptor?.user?.id === expectedUser.id,
    plaintext_absent: (
      !encryptedSource.includes(expectedToken)
      && !encryptedSource.includes(expectedUser.email)
    ),
    credential_recorded: false,
  };
}

function fingerprintEntry(hasher, root, relative = "") {
  const current = relative ? path.join(root, relative) : root;
  let metadata;
  try {
    metadata = lstatSync(current);
  } catch (error) {
    if (error.code === "ENOENT") {
      hasher.update(`absent:${relative}\0`);
      return 0;
    }
    throw error;
  }
  const normalized = relative.split(path.sep).join("/");
  hasher.update(
    `${normalized}\0${metadata.mode & 0o7777}\0${metadata.size}\0${metadata.mtimeMs}\0`,
  );
  if (metadata.isSymbolicLink()) {
    hasher.update("symlink\0");
    return 1;
  }
  if (metadata.isFile()) {
    hasher.update("file\0");
    hasher.update(readFileSync(current));
    return 1;
  }
  if (!metadata.isDirectory()) {
    hasher.update("special\0");
    return 1;
  }
  hasher.update("directory\0");
  let count = 1;
  for (const name of readdirSync(current).sort()) {
    count += fingerprintEntry(hasher, root, relative ? path.join(relative, name) : name);
  }
  return count;
}

export function fingerprintStateRoots(roots = personalRoots) {
  const combined = createHash("sha256");
  const details = [];
  for (const root of roots) {
    const hasher = createHash("sha256");
    const entryCount = fingerprintEntry(hasher, root.path);
    const digest = hasher.digest("hex");
    combined.update(`${root.label}\0${digest}\0`);
    details.push({
      label: root.label,
      entry_count: entryCount,
      digest,
    });
  }
  return {
    digest: combined.digest("hex"),
    entry_count: details.reduce((sum, detail) => sum + detail.entry_count, 0),
    details,
  };
}

function sleep(milliseconds) {
  if (milliseconds > 0) {
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);
  }
}

export function isClarkDesktopRunning() {
  return run("pgrep", ["-x", "clark-desktop"], { timeout_ms: 10_000 }).ok;
}

export function stopClarkDesktop() {
  if (!isClarkDesktopRunning()) return true;
  if (process.platform === "darwin") {
    run(
      "osascript",
      ["-e", 'tell application id "com.clark.desktop.dev" to quit'],
      { timeout_ms: 10_000 },
    );
  }
  for (let attempt = 0; attempt < 40; attempt += 1) {
    if (!isClarkDesktopRunning()) {
      sleep(1_000);
      return true;
    }
    sleep(500);
  }
  // A wedged app must not leak into the user's normal profile restoration.
  // This fallback is intentionally not accepted as graceful lifecycle proof.
  run("pkill", ["-x", "clark-desktop"], { timeout_ms: 10_000 });
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if (!isClarkDesktopRunning()) return false;
    sleep(500);
  }
  return false;
}

export function newDesktopKeyIds(before, after) {
  const prior = new Set(before.map((key) => key.id));
  return after
    .filter(
      (key) => (
        typeof key?.id === "string"
        && key.purpose === "clark_code_desktop"
        && !prior.has(key.id)
      ),
    )
    .map((key) => key.id);
}

async function platformRequest({ origin, token, pathname, method = "GET" }) {
  const response = await fetch(`${new URL(origin).origin}${pathname}`, {
    method,
    headers: {
      authorization: `Bearer ${token}`,
      accept: "application/json",
    },
    signal: AbortSignal.timeout(30_000),
  });
  if (!response.ok) {
    throw new Error(`Clark platform key request failed with HTTP ${response.status}`);
  }
  return response;
}

export async function listPlatformKeys({ origin, token }) {
  const response = await platformRequest({
    origin,
    token,
    pathname: "/api/platform/api-keys",
  });
  const payload = await response.json();
  if (!Array.isArray(payload?.api_keys)) {
    throw new Error("Clark platform key list returned no api_keys array");
  }
  return payload.api_keys;
}

export async function revokePlatformKeys({ origin, token, ids }) {
  let revoked = 0;
  for (const id of ids) {
    if (!/^[0-9a-f-]{36}$/i.test(id)) {
      throw new Error("refusing to revoke a malformed platform key identifier");
    }
    await platformRequest({
      origin,
      token,
      pathname: `/api/platform/api-keys/${id}`,
      method: "DELETE",
    });
    revoked += 1;
  }
  return revoked;
}
