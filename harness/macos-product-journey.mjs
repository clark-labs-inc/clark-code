#!/usr/bin/env node

import { createHash, randomUUID } from "node:crypto";
import {
  accessSync,
  chmodSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

import { mintClarkQaSession } from "./clark-qa-auth.mjs";
import {
  captureMacosProductWindow,
  clickMacosProductText,
} from "./macos-product-observation.mjs";
import {
  MACOS_APP_BUNDLE,
  MACOS_QA_DATA_STORE_UUID,
  MACOS_QA_MODEL,
  MACOS_QA_REMOTE_HOST,
  MACOS_QA_REMOTE_ROOT,
  MACOS_QA_WINDOW_TITLE,
  assertTargetOutputPath,
  buildStoreHelper,
  fingerprintStateRoots,
  isClarkDesktopRunning,
  listPlatformKeys,
  newDesktopKeyIds,
  probeNativeCredentialState,
  redact,
  repoDir,
  revokePlatformKeys,
  run,
  runStoreHelper,
  stopClarkDesktop,
  writeBootstrap,
  writeNativeCredentialBootstrap,
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
    || window?.create !== false
    || window?.dataStoreIdentifier !== undefined
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

function resolveQaRemoteEndpoint() {
  const resolved = run("ssh", ["-G", MACOS_QA_REMOTE_HOST], { timeout_ms: 30_000 });
  const ready = run(
    "ssh",
    ["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", MACOS_QA_REMOTE_HOST, "true"],
    { timeout_ms: 15_000 },
  );
  if (!resolved.ok || !ready.ok) {
    throw new Error("the configured QA SSH host is not ready");
  }
  return { host: MACOS_QA_REMOTE_HOST };
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
      CLARK_LOGS: path.join(qaHome, "logs"),
      CLARK_CAPTURED_LOGS: path.join(qaHome, "captured"),
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

function runtimeEvidence(qaHome) {
  const logDirectory = path.join(qaHome, "logs");
  const connects = [];
  const connectFailures = [];
  const bootstrapFailures = [];
  const shutdowns = [];
  if (existsSync(logDirectory)) {
    for (const name of readdirSync(logDirectory)) {
      if (!name.startsWith("clark-desktop") || !name.endsWith("jsonl")) continue;
      const contents = readFileSync(path.join(logDirectory, name), "utf8");
      for (const line of contents.split(/\r?\n/)) {
        if (!line) continue;
        let event;
        try {
          event = JSON.parse(line);
        } catch {
          continue;
        }
        if (event.event === "remote_worker_connected") {
          connects.push({
            connection_kind: event.connection_kind,
            connect_duration_ms: event.connect_duration_ms,
            account_worker_count: event.account_worker_count,
            ssh_transport: event.ssh_transport,
            worker_arch: event.worker_arch,
          });
        } else if (event.event === "remote_worker_connect_failed") {
          connectFailures.push({ stage: event.stage });
        } else if (event.event === "remote_worker_bootstrap_failed") {
          bootstrapFailures.push({ category: event.category });
        } else if (event.event === "runtime_registry_shutdown") {
          shutdowns.push({
            sessions_shutdown: event.sessions,
            workers_shutdown: event.workers,
            terminals_shutdown: event.terminals,
          });
        }
      }
    }
  }
  const passed = (
    connects.length === 2
    && connects.every((event) => (
      event.connection_kind === "started"
      && event.account_worker_count === 1
      && event.ssh_transport === "control_master"
      && event.worker_arch === "linux-x86_64"
    ))
    && shutdowns.length === 2
    && shutdowns.every((event) => event.workers_shutdown === 1)
  );
  return {
    status: passed ? "passed" : "failed",
    expected_process_connects: 2,
    observed_process_connects: connects.length,
    warm_reselection_added_connect: connects.length !== 2,
    stable_account_worker_count: connects.every(
      (event) => event.account_worker_count === 1,
    ),
    connects,
    connect_failures: connectFailures,
    bootstrap_failures: bootstrapFailures,
    shutdowns,
    identifiers_recorded: false,
  };
}

function waitForRuntimeConnects(qaHome, expectedCount, timeoutMs = 30_000) {
  const started = Date.now();
  let evidence = runtimeEvidence(qaHome);
  while (evidence.connects.length < expectedCount && Date.now() - started < timeoutMs) {
    sleep(250);
    evidence = runtimeEvidence(qaHome);
  }
  return {
    status: evidence.connects.length >= expectedCount ? "passed" : "failed",
    expected_count: expectedCount,
    observed_count: evidence.connects.length,
    duration_ms: Date.now() - started,
  };
}

function clickUntilRuntimeConnect(
  screenshotPath,
  name,
  qaHome,
  expectedCount,
) {
  const started = Date.now();
  const initialScreenshot = path.isAbsolute(screenshotPath)
    ? screenshotPath
    : path.join(repoDir, screenshotPath);
  const evidenceRoot = path.dirname(path.dirname(initialScreenshot));
  let currentScreenshot = screenshotPath;
  let selection = null;
  let wait = null;
  let observation = null;
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    selection = clickMacosElement(currentScreenshot, name);
    if (selection.status === "passed") {
      wait = waitForRuntimeConnects(qaHome, expectedCount, 10_000);
      if (wait.status === "passed") {
        return {
          selection: {
            ...selection,
            duration_ms: Date.now() - started,
            attempts: attempt,
          },
          wait,
        };
      }
      observation = captureMacosProductWindow(
        path.join(evidenceRoot, `selection-attempt-${attempt}`),
      );
      if (observation.screenshot) currentScreenshot = observation.screenshot;
    }
  }
  return {
    selection: {
      ...selection,
      status: "failed",
      duration_ms: Date.now() - started,
      attempts: 3,
    },
    wait,
    observation,
  };
}

function observeProductReady(outputDir, conversationTitle = null) {
  const started = Date.now();
  let observation = null;
  for (let attempt = 1; attempt <= 10; attempt += 1) {
    observation = captureMacosProductWindow(outputDir, conversationTitle);
    if (observation.status === "passed") {
      return {
        ...observation,
        ready_duration_ms: Date.now() - started,
        attempts: attempt,
      };
    }
    sleep(500);
  }
  return {
    ...observation,
    ready_duration_ms: Date.now() - started,
    attempts: 10,
  };
}

async function createDisposableConversation({
  issuer,
  token,
  id,
  title,
  project,
  remoteHost,
}) {
  const response = await fetch(
    `${issuer}/api/desktop/conversations/${encodeURIComponent(id)}`,
    {
      method: "PUT",
      headers: {
        authorization: `Bearer ${token}`,
        "content-type": "application/json",
      },
      body: JSON.stringify({
        title,
        provider: "local",
        project,
        repositoryFingerprint: null,
        remoteHost,
        mode: "ask",
        titleLocked: true,
        rev: 0,
        snapshot: {
          starting: false,
          runs: {},
          timeline: [],
          tool_calls: {},
          artifacts: [],
          provider_incidents: {},
        },
        status: "idle",
        baseRev: 0,
        mutationId: randomUUID(),
      }),
    },
  );
  if (!response.ok) {
    throw new Error(`could not create disposable conversation (${response.status})`);
  }
}

async function deleteDisposableConversation({ issuer, token, id }) {
  const response = await fetch(
    `${issuer}/api/desktop/conversations/${encodeURIComponent(id)}`,
    {
      method: "DELETE",
      headers: { authorization: `Bearer ${token}` },
    },
  );
  if (!response.ok && response.status !== 404) {
    throw new Error(`could not delete disposable conversation (${response.status})`);
  }
}

function clickMacosElement(screenshotPath, name) {
  const started = Date.now();
  const clicked = clickMacosProductText(screenshotPath, name);
  return {
    ...clicked,
    duration_ms: Date.now() - started,
    attempts: 1,
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
  const remoteEndpoint = resolveQaRemoteEndpoint();
  const { qaHome, workspaceRoot, workspace } = createProfileLayout(resolvedOutput, marker);
  const helper = buildStoreHelper(resolvedOutput);
  const appWasRunning = isClarkDesktopRunning();
  const source = sourceRevision();
  let build = null;
  let minted = null;
  let seed = null;
  const launches = { initial: null, restart: null };
  const runtimeConnectWaits = { initial: null, restart: null };
  const observations = { initial: null, restart: null };
  const conversationId = randomUUID();
  const conversationTitle = "Clark Reconnect QA";
  const conversationSelections = {
    initial: null,
    detach: null,
    warm: null,
    restart: null,
  };
  let conversationCreated = false;
  let conversationCleanup = { status: "not_needed", identifiers_recorded: false };
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
  let nativeRuntime = { status: "pending" };
  let failure = null;
  const bootstrapPath = path.join(resolvedOutput, "profile-bootstrap.json");

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
      token: minted.retained_auth.clarkToken,
    });
    writeBootstrap(bootstrapPath, {
      cwd: workspace,
      model: MACOS_QA_MODEL,
      marker,
      accountScope: `id:${minted.account.id.toLowerCase()}`,
      remoteHost: remoteEndpoint.host,
      remoteRoot: MACOS_QA_REMOTE_ROOT,
    });
    writeNativeCredentialBootstrap(qaHome, minted.retained_auth);
    await createDisposableConversation({
      issuer: minted.issuer,
      token: minted.retained_auth.clarkToken,
      id: conversationId,
      title: conversationTitle,
      project: MACOS_QA_REMOTE_ROOT,
      remoteHost: remoteEndpoint.host,
    });
    conversationCreated = true;
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

    launches.initial = launcher("--qa-launch", qaHome, 120_000);
    if (launches.initial.status !== "passed") throw new Error(launches.initial.error);
    observations.initial = observeProductReady(path.join(resolvedOutput, "initial"));
    if (observations.initial.status !== "passed") {
      throw new Error(observations.initial.error || "initial macOS product visual contract failed");
    }
    const initialSelection = clickUntilRuntimeConnect(
      observations.initial.screenshot,
      conversationTitle,
      qaHome,
      1,
    );
    conversationSelections.initial = initialSelection.selection;
    runtimeConnectWaits.initial = initialSelection.wait;
    if (initialSelection.observation) {
      observations.initial_selection_failure = initialSelection.observation;
    }
    if (conversationSelections.initial.status !== "passed") {
      throw new Error(
        conversationSelections.initial.error
        || "initial conversation selection did not admit a remote worker",
      );
    }
    if (runtimeConnectWaits.initial.status !== "passed") {
      throw new Error("initial remote worker did not become ready");
    }
    observations.initial_conversation = observeProductReady(
      path.join(resolvedOutput, "initial-conversation"),
      conversationTitle,
    );
    if (observations.initial_conversation.status !== "passed") {
      throw new Error("initial existing-conversation open did not become ready");
    }
    conversationSelections.detach = clickMacosElement(
      observations.initial_conversation.screenshot,
      "New session",
    );
    if (conversationSelections.detach.status !== "passed") {
      throw new Error(conversationSelections.detach.error);
    }
    observations.detached = observeProductReady(path.join(resolvedOutput, "detached"));
    if (observations.detached.status !== "passed") {
      throw new Error("new-session surface did not become ready after detaching conversation");
    }
    conversationSelections.warm = clickMacosElement(
      observations.detached.screenshot,
      conversationTitle,
    );
    if (conversationSelections.warm.status !== "passed") {
      throw new Error(conversationSelections.warm.error);
    }
    observations.warm_conversation = observeProductReady(
      path.join(resolvedOutput, "warm-conversation"),
      conversationTitle,
    );
    if (observations.warm_conversation.status !== "passed") {
      throw new Error("warm existing-conversation reuse did not become ready");
    }
    if (!stopClarkDesktop()) throw new Error("could not stop the isolated QA app");

    launches.restart = launcher("--qa-launch", qaHome, 120_000);
    if (launches.restart.status !== "passed") throw new Error(launches.restart.error);
    observations.restart = observeProductReady(path.join(resolvedOutput, "restart"));
    if (observations.restart.status !== "passed") {
      throw new Error(observations.restart.error || "restarted macOS product visual contract failed");
    }
    const restartSelection = clickUntilRuntimeConnect(
      observations.restart.screenshot,
      conversationTitle,
      qaHome,
      2,
    );
    conversationSelections.restart = restartSelection.selection;
    runtimeConnectWaits.restart = restartSelection.wait;
    if (restartSelection.observation) {
      observations.restart_selection_failure = restartSelection.observation;
    }
    if (conversationSelections.restart.status !== "passed") {
      throw new Error(
        conversationSelections.restart.error
        || "restarted conversation selection did not admit a remote worker",
      );
    }
    if (runtimeConnectWaits.restart.status !== "passed") {
      throw new Error("restarted remote worker did not become ready");
    }
    observations.restart_conversation = observeProductReady(
      path.join(resolvedOutput, "restart-conversation"),
      conversationTitle,
    );
    if (observations.restart_conversation.status !== "passed") {
      throw new Error("restarted existing-conversation open did not become ready");
    }
    if (!stopClarkDesktop()) throw new Error("could not stop the restarted isolated QA app");

    profileProbe = runStoreHelper({
      helperPath: helper.executable_path,
      qaHome,
      operation: "probe",
      args: [
        workspace,
        MACOS_QA_MODEL,
        marker,
        `id:${minted.account.id.toLowerCase()}`,
        remoteEndpoint.host,
        MACOS_QA_REMOTE_ROOT,
      ],
    });
    if (profileProbe.status !== "passed") {
      throw new Error(profileProbe.error || "isolated QA profile probe failed");
    }
    const credentialProbe = probeNativeCredentialState(qaHome, minted.retained_auth);
    if (credentialProbe.status !== "passed") {
      throw new Error("isolated native credential probe failed");
    }
    profileProbe.native_credentials = credentialProbe;

    const keysAfter = await listPlatformKeys({
      origin: minted.issuer,
      token: minted.retained_auth.clarkToken,
    });
    newKeyIds = newDesktopKeyIds(keysBefore, keysAfter);
    if (newKeyIds.length !== 1) {
      throw new Error(`expected one disposable desktop key; observed ${newKeyIds.length}`);
    }
  } catch (error) {
    failure = redact(error?.message || error);
  } finally {
    stopClarkDesktop();
    nativeRuntime = runtimeEvidence(qaHome);
    if (minted) {
      if (conversationCreated) {
        try {
          await deleteDisposableConversation({
            issuer: minted.issuer,
            token: minted.retained_auth.clarkToken,
            id: conversationId,
          });
          conversationCleanup = { status: "passed", identifiers_recorded: false };
        } catch (error) {
          conversationCleanup = {
            status: "failed",
            identifiers_recorded: false,
            error: redact(error?.message || error),
          };
        }
      }
      try {
        if (!newKeyIds.length) {
          const keysAfter = await listPlatformKeys({
            origin: minted.issuer,
            token: minted.retained_auth.clarkToken,
          });
          newKeyIds = newDesktopKeyIds(keysBefore, keysAfter);
        }
        const revoked = await revokePlatformKeys({
          origin: minted.issuer,
          token: minted.retained_auth.clarkToken,
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
    && launches.initial?.status === "passed"
    && launches.restart?.status === "passed"
    && runtimeConnectWaits.initial?.status === "passed"
    && runtimeConnectWaits.restart?.status === "passed"
    && observations.initial?.status === "passed"
    && observations.restart?.status === "passed"
    && observations.initial_conversation?.status === "passed"
    && observations.detached?.status === "passed"
    && observations.warm_conversation?.status === "passed"
    && observations.restart_conversation?.status === "passed"
    && conversationSelections.initial?.status === "passed"
    && conversationSelections.detach?.status === "passed"
    && conversationSelections.warm?.status === "passed"
    && conversationSelections.restart?.status === "passed"
    && profileProbe?.status === "passed"
    && customStoreContained
    && keyCleanup.status === "passed"
    && conversationCleanup.status === "passed"
    && profileCleanup.status === "passed"
    && nativeRuntime.status === "passed"
    && personalStateUnchanged
    && restoration?.status === "passed"
    && restoration?.previous_running_state_restored === true
  );
  const receipt = {
    schema_version: 2,
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
          email_domain: minted.account.email.split("@").at(-1).toLowerCase(),
          issuer: minted.issuer,
          expires_in_seconds_at_mint: minted.expires_in_seconds,
          transport: "better_auth_email_to_short_lived_jwt",
        }
      : null,
    build,
    seed,
    launches,
    runtime_connect_waits: runtimeConnectWaits,
    profile_probe: profileProbe,
    observations,
    native_runtime: nativeRuntime,
    conversation: {
      created: conversationCreated,
      title_recorded: false,
      identifier_recorded: false,
      selections: conversationSelections,
    },
    cleanup: {
      profile: profileCleanup,
      platform_key: keyCleanup,
      conversation: conversationCleanup,
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
    restart_ready: receipt.observations.restart?.status === "passed",
    existing_conversation_reopened:
      receipt.observations.restart_conversation?.status === "passed",
    warm_conversation_reused:
      receipt.observations.warm_conversation?.status === "passed",
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
session, verifies the real authenticated product UI before and after a full
process restart plus the same-account provider
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
