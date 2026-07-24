#!/usr/bin/env node

import { createHash, randomUUID } from "node:crypto";
import {
  accessSync,
  chmodSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

import { mintClarkQaSession } from "./clark-qa-auth.mjs";
import { captureMacosProductWindow } from "./macos-product-observation.mjs";
import {
  MACOS_APP_BUNDLE,
  MACOS_QA_DATA_STORE_BYTES,
  MACOS_QA_DATA_STORE_UUID,
  MACOS_QA_MODEL,
  MACOS_QA_WINDOW_TITLE,
  assertTargetOutputPath,
  buildStoreHelper,
  fingerprintStateRoots,
  isClarkDesktopRunning,
  listPlatformKeys,
  newDesktopKeyIds,
  redact,
  repoDir,
  revokePlatformKeys,
  run,
  runStoreHelper,
  stopClarkDesktop,
  writeBootstrap,
} from "./macos-qa-profile.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const qaConfigPath = path.join(
  repoDir,
  "src-tauri",
  "tauri.qa.macos.conf.json",
);
const launcherPath = path.join(repoDir, "script", "build_and_run.sh");

function sleep(milliseconds) {
  if (milliseconds > 0) {
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);
  }
}

function prepareOutput(outputDir) {
  const resolved = assertTargetOutputPath(outputDir);
  try {
    accessSync(resolved);
    throw new Error(`refusing to overwrite macOS journey output ${resolved}`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  mkdirSync(resolved, { recursive: true, mode: 0o700 });
  chmodSync(resolved, 0o700);
  return resolved;
}

function validateQaConfig() {
  const config = JSON.parse(readFileSync(qaConfigPath, "utf8"));
  const window = config?.app?.windows?.[0];
  if (
    window?.title !== MACOS_QA_WINDOW_TITLE
    || JSON.stringify(window?.dataStoreIdentifier) !== JSON.stringify(MACOS_QA_DATA_STORE_BYTES)
  ) {
    throw new Error("macOS QA Tauri config does not match the pinned profile contract");
  }
}

function sourceRevision() {
  const revision = run("git", ["rev-parse", "HEAD"]);
  const dirty = run("git", ["status", "--porcelain"]);
  return {
    revision: revision.ok ? revision.stdout.trim() : "unknown",
    dirty: !dirty.ok || Boolean(dirty.stdout.trim()),
  };
}

function fileSha256(filePath) {
  return createHash("sha256").update(readFileSync(filePath)).digest("hex");
}

function createProfileLayout(outputDir, marker) {
  const qaHome = path.join(outputDir, "profile", "home");
  const workspaceRoot = path.join(
    repoDir,
    "target",
    "macos-qa-workspaces",
    marker,
  );
  const workspace = path.join(workspaceRoot, "ClarkCodeQA");
  mkdirSync(path.join(qaHome, "tmp"), { recursive: true, mode: 0o700 });
  mkdirSync(workspace, { recursive: true, mode: 0o700 });
  chmodSync(path.join(outputDir, "profile"), 0o700);
  chmodSync(qaHome, 0o700);
  chmodSync(workspaceRoot, 0o700);
  chmodSync(workspace, 0o700);
  writeFileSync(
    path.join(workspace, "README.md"),
    "# Clark Code macOS QA fixture\n\nThis disposable workspace is owned by the autonomous benchmark.\n",
    { mode: 0o600 },
  );
  return { qaHome, workspaceRoot, workspace };
}

function launcher(command, qaHome, timeoutMs = 3_600_000) {
  const completed = run(launcherPath, [command], {
    env: {
      ...process.env,
      CLARK_QA_MACOS_HOME: qaHome,
    },
    timeout_ms: timeoutMs,
    max_buffer: 64 * 1024 * 1024,
  });
  return {
    status: completed.ok ? "passed" : "failed",
    duration_ms: completed.duration_ms,
    error: completed.ok
      ? null
      : redact(completed.stderr || completed.stdout).slice(-4_000),
  };
}

function restoreNormalApp(wasRunning) {
  const command = wasRunning ? "--verify" : "--build";
  const restored = run(launcherPath, [command], {
    timeout_ms: 3_600_000,
    max_buffer: 64 * 1024 * 1024,
  });
  return {
    status: restored.ok ? "passed" : "failed",
    previous_running_state_restored: wasRunning
      ? isClarkDesktopRunning()
      : !isClarkDesktopRunning(),
    duration_ms: restored.duration_ms,
    error: restored.ok
      ? null
      : redact(restored.stderr || restored.stdout).slice(-4_000),
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

export async function runMacosAuthenticatedSmoke({ outputDir }) {
  if (process.platform !== "darwin") {
    throw new Error("macOS product journey requires the macOS host");
  }
  validateQaConfig();
  const resolvedOutput = prepareOutput(outputDir);
  const marker = randomUUID();
  const { qaHome, workspaceRoot, workspace } = createProfileLayout(
    resolvedOutput,
    marker,
  );
  const helper = buildStoreHelper(resolvedOutput);
  const appWasRunning = isClarkDesktopRunning();
  const source = sourceRevision();
  let build = null;
  let minted = null;
  let seed = null;
  let launch = null;
  let observation = null;
  let profileProbe = null;
  let customStoreContained = false;
  let beforePersonal = null;
  let afterPersonal = null;
  let keysBefore = [];
  let newKeyIds = [];
  let keyCleanup = {
    status: "not_needed",
    created_count: 0,
    revoked_count: 0,
    identifiers_recorded: false,
  };
  let profileCleanup = {
    status: "pending",
    qa_home_removed: false,
    bootstrap_erased: false,
  };
  let restoration = null;
  let failure = null;
  const bootstrapPath = path.join(resolvedOutput, "transient-auth.json");

  try {
    build = launcher("--qa-build", qaHome);
    if (build.status !== "passed") throw new Error(build.error);
    const appBinary = path.join(MACOS_APP_BUNDLE, "Contents", "MacOS", "clark-desktop");
    build.app_binary_sha256 = fileSha256(appBinary);
    build.strict_signature_verified = true;

    if (!stopClarkDesktop()) throw new Error("could not quiesce the existing Clark process");
    beforePersonal = fingerprintStateRoots();

    minted = await mintClarkQaSession();
    keysBefore = await listPlatformKeys({
      origin: minted.issuer,
      token: minted.session.clark.token,
    });
    writeBootstrap(bootstrapPath, {
      auth_session: minted.session,
      cwd: workspace,
      model: MACOS_QA_MODEL,
      marker,
    });
    seed = runStoreHelper({
      helperPath: helper.executable_path,
      qaHome,
      operation: "seed",
      args: [bootstrapPath],
    });
    if (seed.status !== "passed") throw new Error(seed.error || "QA profile seed failed");
    customStoreContained = existsSync(
      path.join(
        qaHome,
        "Library",
        "WebKit",
        "com.clark.desktop.dev",
        "WebsiteDataStore",
      ),
    );
    if (!customStoreContained) {
      throw new Error("custom WebKit data store escaped the isolated QA home");
    }
    unlinkSync(bootstrapPath);
    profileCleanup.bootstrap_erased = !existsSync(bootstrapPath);

    launch = launcher("--qa-launch", qaHome, 120_000);
    if (launch.status !== "passed") throw new Error(launch.error);
    sleep(30_000);
    observation = captureMacosProductWindow(resolvedOutput);
    if (observation.status !== "passed") {
      throw new Error(observation.error || "macOS product visual contract failed");
    }
    if (!stopClarkDesktop()) throw new Error("could not stop the isolated QA app");

    profileProbe = runStoreHelper({
      helperPath: helper.executable_path,
      qaHome,
      operation: "probe",
      args: [
        minted.account_fingerprint,
        workspace,
        MACOS_QA_MODEL,
        marker,
      ],
    });
    if (profileProbe.status !== "passed") {
      throw new Error(profileProbe.error || "isolated QA profile probe failed");
    }

    const keysAfter = await listPlatformKeys({
      origin: minted.issuer,
      token: minted.session.clark.token,
    });
    newKeyIds = newDesktopKeyIds(keysBefore, keysAfter);
    if (newKeyIds.length !== 1) {
      throw new Error(`expected one disposable desktop key; observed ${newKeyIds.length}`);
    }
  } catch (error) {
    failure = redact(error?.message || error);
  } finally {
    stopClarkDesktop();
    if (minted) {
      try {
        if (!newKeyIds.length) {
          const keysAfter = await listPlatformKeys({
            origin: minted.issuer,
            token: minted.session.clark.token,
          });
          newKeyIds = newDesktopKeyIds(keysBefore, keysAfter);
        }
        const revoked = await revokePlatformKeys({
          origin: minted.issuer,
          token: minted.session.clark.token,
          ids: newKeyIds,
        });
        keyCleanup = {
          status: revoked === newKeyIds.length ? "passed" : "failed",
          created_count: newKeyIds.length,
          revoked_count: revoked,
          identifiers_recorded: false,
        };
      } catch (error) {
        keyCleanup = {
          status: "failed",
          created_count: newKeyIds.length,
          revoked_count: 0,
          identifiers_recorded: false,
          error: redact(error?.message || error),
        };
      }
    }
    if (existsSync(bootstrapPath)) unlinkSync(bootstrapPath);
    profileCleanup.bootstrap_erased = !existsSync(bootstrapPath);
    rmSync(qaHome, { recursive: true, force: true });
    profileCleanup.qa_home_removed = !existsSync(qaHome);
    profileCleanup.status = (
      profileCleanup.qa_home_removed && profileCleanup.bootstrap_erased
    ) ? "passed" : "failed";
    if (beforePersonal) afterPersonal = fingerprintStateRoots();
    rmSync(workspaceRoot, { recursive: true, force: true });
    restoration = restoreNormalApp(appWasRunning);
  }

  const personalStateUnchanged = Boolean(
    beforePersonal
    && afterPersonal
    && beforePersonal.digest === afterPersonal.digest,
  );
  const passed = (
    !failure
    && build?.status === "passed"
    && seed?.status === "passed"
    && launch?.status === "passed"
    && observation?.status === "passed"
    && profileProbe?.status === "passed"
    && customStoreContained
    && keyCleanup.status === "passed"
    && profileCleanup.status === "passed"
    && personalStateUnchanged
    && restoration?.status === "passed"
    && restoration?.previous_running_state_restored === true
  );
  const receipt = {
    schema_version: 1,
    benchmark: "clark_code_macos_authenticated_product_smoke",
    status: passed ? "passed" : "failed",
    generated_at: new Date().toISOString(),
    source_revision: source.revision,
    source_dirty: source.dirty,
    platform: "macos",
    virtualization: "host_native",
    required_user_vm_actions: 0,
    manual_vm_actions_allowed: false,
    human_input_observed: false,
    credential_recorded: false,
    paid_calls_made: false,
    model: MACOS_QA_MODEL,
    profile: {
      isolation: "qa_home_plus_custom_wkwebsite_data_store",
      data_store_uuid: MACOS_QA_DATA_STORE_UUID,
      custom_store_contained_in_qa_home: customStoreContained,
      personal_state_unchanged: personalStateUnchanged,
      personal_state_entries_before: beforePersonal?.entry_count ?? null,
      personal_state_entries_after: afterPersonal?.entry_count ?? null,
      personal_state_digests_recorded: false,
    },
    auth: minted
      ? {
          account_fingerprint: minted.account_fingerprint,
          email_domain: minted.session.user.email.split("@").at(-1).toLowerCase(),
          issuer: minted.issuer,
          expires_in_seconds_at_mint: minted.expires_in_seconds,
          transport: "better_auth_email_to_short_lived_jwt",
        }
      : null,
    build,
    seed,
    launch,
    profile_probe: profileProbe,
    observation,
    cleanup: {
      profile: profileCleanup,
      platform_key: keyCleanup,
      workspace_removed: !existsSync(workspaceRoot),
      normal_app_restoration: restoration,
    },
    failure,
  };
  const receiptPath = path.join(resolvedOutput, "receipt.json");
  writeFileSync(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o600 });
  console.log(JSON.stringify({
    status: receipt.status,
    auth_domain: receipt.auth?.email_domain || null,
    isolated_profile: receipt.profile.personal_state_unchanged,
    workspace_ready: receipt.profile_probe?.status === "passed",
    provider_key_revoked: receipt.cleanup.platform_key.status === "passed",
    profile_erased: receipt.cleanup.profile.status === "passed",
    personal_app_restored:
      receipt.cleanup.normal_app_restoration?.previous_running_state_restored === true,
    required_user_vm_actions: 0,
  }));
  console.log(`RECEIPT=${receiptPath}`);
  if (!passed) process.exitCode = 1;
  return receipt;
}

async function runCli() {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    console.log(`Autonomous macOS Clark Code product journey

Usage:
  node harness/macos-product-journey.mjs auth-smoke [--out NEW_TARGET_DIRECTORY]

The journey builds the exact signed development app with a dedicated
WKWebsiteDataStore and disposable HOME, mints a Clark-owned short-lived
session, verifies the real authenticated product UI and same-account provider
key, revokes the disposable key, erases the profile, proves the personal Clark
state did not change, and restores the prior normal-app running state.`);
    return;
  }
  if (args[0] !== "auth-smoke") {
    throw new Error(`unknown command ${JSON.stringify(args[0])}`);
  }
  for (let index = 1; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--out") {
      index += 1;
      continue;
    }
    if (arg.startsWith("--out=")) continue;
    throw new Error(`unknown argument ${JSON.stringify(arg)}`);
  }
  const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
  const outputDir = path.resolve(
    repoDir,
    valueArg(args, "--out")
      || path.join("target", "macos-product-journey", `${stamp}-${process.pid}`),
  );
  await runMacosAuthenticatedSmoke({ outputDir });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await runCli();
}
