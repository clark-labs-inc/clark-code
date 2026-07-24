#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  accessSync,
  chmodSync,
  copyFileSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

import { secureOwnerOnlyFile } from "./owner-only-file.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const featureMap = JSON.parse(
  readFileSync(path.join(harnessDir, "clark-code-feature-map.json"), "utf8"),
);
const inventory = JSON.parse(
  readFileSync(path.join(harnessDir, "clark-code-capability-inventory.json"), "utf8"),
);
const PLATFORM_BY_HOST = { darwin: "macos", win32: "windows", linux: "ubuntu" };
const ALLOWED_EVIDENCE_KINDS = new Set([
  "artifact",
  "log",
  "receipt",
  "screenshot",
  "video",
]);
const MAX_EVIDENCE_BYTES = 50 * 1024 * 1024;

function duplicates(values) {
  const seen = new Set();
  const repeated = new Set();
  for (const value of values) {
    if (seen.has(value)) repeated.add(value);
    seen.add(value);
  }
  return [...repeated];
}

export function expectedScenarios(platform) {
  if (!featureMap.platforms.includes(platform)) throw new Error(`unknown platform ${platform}`);
  return [
    ...featureMap.real_use_scenarios[platform],
    ...inventory.real_use_scenarios[platform],
  ];
}

export function sha256File(filePath) {
  const hash = createHash("sha256");
  hash.update(readFileSync(filePath));
  return hash.digest("hex");
}

function assertNoSecrets(value, location = "$") {
  if (typeof value === "string") {
    if (
      /\bck_(?:live|test)_[A-Za-z0-9._-]+\b/.test(value)
      || /\bsk-[A-Za-z0-9_-]{16,}\b/.test(value)
      || /authorization\s*[:=]\s*bearer\s+\S+/i.test(value)
    ) {
      throw new Error(`${location} contains a credential-shaped value`);
    }
    return;
  }
  if (Array.isArray(value)) {
    value.forEach((item, index) => assertNoSecrets(item, `${location}[${index}]`));
    return;
  }
  if (!value || typeof value !== "object") return;
  for (const [key, child] of Object.entries(value)) {
    if (
      /(api.?key|password|client.?secret|access.?token|refresh.?token|authorization)/i.test(key)
      && child !== null
      && child !== false
      && child !== ""
    ) {
      throw new Error(`${location}.${key} must not record credential material`);
    }
    assertNoSecrets(child, `${location}.${key}`);
  }
}

function evidenceFile(baseDir, relativePath) {
  if (
    typeof relativePath !== "string"
    || !relativePath
    || path.isAbsolute(relativePath)
  ) {
    throw new Error(`evidence path must be a non-empty relative path: ${relativePath}`);
  }
  const base = realpathSync(baseDir);
  const candidate = path.resolve(base, relativePath);
  if (candidate !== base && !candidate.startsWith(`${base}${path.sep}`)) {
    throw new Error(`evidence path escapes the observation directory: ${relativePath}`);
  }
  const metadata = lstatSync(candidate);
  if (metadata.isSymbolicLink() || !metadata.isFile()) {
    throw new Error(`evidence must be a regular non-symlink file: ${relativePath}`);
  }
  const canonical = realpathSync(candidate);
  if (canonical !== base && !canonical.startsWith(`${base}${path.sep}`)) {
    throw new Error(`evidence resolves outside the observation directory: ${relativePath}`);
  }
  if (metadata.size < 1 || metadata.size > MAX_EVIDENCE_BYTES) {
    throw new Error(`evidence size is outside 1..${MAX_EVIDENCE_BYTES}: ${relativePath}`);
  }
  return { canonical, size_bytes: metadata.size };
}

export function validateObservation(raw, platform, receiptPath) {
  assertNoSecrets(raw);
  if (raw?.schema_version !== 1 || raw?.benchmark !== "clark_code_real_use_observation") {
    throw new Error("observation must use clark_code_real_use_observation schema version 1");
  }
  if (raw.platform !== platform) {
    throw new Error(`observation platform ${raw.platform} does not match ${platform}`);
  }
  if (raw.credential_recorded !== false) {
    throw new Error("observation must explicitly record credential_recorded=false");
  }
  if (
    raw.required_user_vm_actions !== 0
    || raw.manual_vm_actions_allowed !== false
    || raw.human_input_observed !== false
  ) {
    throw new Error(
      "observation must prove zero required user actions, disallow manual VM actions, and record no human input",
    );
  }
  if (!raw.source_revision || typeof raw.source_revision !== "string") {
    throw new Error("observation requires a source_revision");
  }
  if (!raw.environment || typeof raw.environment.gui_visible !== "boolean") {
    throw new Error("observation requires environment.gui_visible");
  }
  const expectedVm = inventory.real_use_environments[platform].vm_name;
  if (expectedVm && raw.environment.vm_name !== expectedVm) {
    throw new Error(`observation must name the exact VM ${JSON.stringify(expectedVm)}`);
  }
  if (!Array.isArray(raw.scenarios)) throw new Error("observation scenarios must be an array");
  const expected = expectedScenarios(platform);
  const expectedIds = expected.map((scenario) => scenario.id);
  const actualIds = raw.scenarios.map((scenario) => scenario.id);
  const repeated = duplicates(actualIds);
  if (repeated.length) throw new Error(`duplicate observation scenarios: ${repeated.join(", ")}`);
  const missing = expectedIds.filter((id) => !actualIds.includes(id));
  const unknown = actualIds.filter((id) => !expectedIds.includes(id));
  if (missing.length || unknown.length) {
    throw new Error(
      `observation scenario mismatch; missing=[${missing.join(", ")}] unknown=[${unknown.join(", ")}]`,
    );
  }

  const baseDir = path.dirname(path.resolve(receiptPath));
  const scenarios = expected.map((contract) => {
    const observed = raw.scenarios.find((scenario) => scenario.id === contract.id);
    if (!["observed", "failed", "blocked"].includes(observed.status)) {
      throw new Error(`${contract.id} has invalid observation status ${observed.status}`);
    }
    if (!Array.isArray(observed.assertions)) {
      throw new Error(`${contract.id} assertions must be an array`);
    }
    if (!Array.isArray(observed.evidence)) {
      throw new Error(`${contract.id} evidence must be an array`);
    }
    if (observed.status === "observed") {
      if (!raw.environment.gui_visible) {
        throw new Error(`${contract.id} cannot be observed without a visible GUI`);
      }
      if (!observed.assertions.length || !observed.evidence.length) {
        throw new Error(`${contract.id} observed status requires assertions and evidence`);
      }
      if (observed.assertions.some((assertion) => assertion.status !== "passed")) {
        throw new Error(`${contract.id} observed status requires every assertion to pass`);
      }
    } else if (!observed.finding || typeof observed.finding !== "string") {
      throw new Error(`${contract.id} ${observed.status} status requires a finding`);
    }
    const assertionIds = observed.assertions.map((assertion) => assertion.id);
    const evidenceIds = observed.evidence.map((evidence) => evidence.id);
    if (duplicates(assertionIds).length || assertionIds.some((id) => !id)) {
      throw new Error(`${contract.id} assertion ids must be non-empty and unique`);
    }
    if (duplicates(evidenceIds).length || evidenceIds.some((id) => !id)) {
      throw new Error(`${contract.id} evidence ids must be non-empty and unique`);
    }
    const evidence = observed.evidence.map((item) => {
      if (!ALLOWED_EVIDENCE_KINDS.has(item.kind)) {
        throw new Error(`${contract.id}/${item.id} has invalid evidence kind ${item.kind}`);
      }
      if (!/^[a-f0-9]{64}$/.test(item.sha256 || "")) {
        throw new Error(`${contract.id}/${item.id} requires a lowercase SHA-256`);
      }
      const file = evidenceFile(baseDir, item.path);
      const actualHash = sha256File(file.canonical);
      if (actualHash !== item.sha256) {
        throw new Error(`${contract.id}/${item.id} SHA-256 does not match ${item.path}`);
      }
      return {
        id: item.id,
        kind: item.kind,
        source_path: item.path,
        sha256: actualHash,
        size_bytes: file.size_bytes,
        canonical: file.canonical,
      };
    });
    return {
      id: contract.id,
      status: observed.status,
      finding: observed.finding || null,
      assertions: observed.assertions.map(({ id, status, evidence: detail }) => ({
        id,
        status,
        evidence: detail || null,
      })),
      evidence,
    };
  });
  return {
    platform,
    source_revision: raw.source_revision,
    environment: raw.environment,
    generated_at: raw.generated_at || null,
    required_user_vm_actions: 0,
    manual_vm_actions_allowed: false,
    human_input_observed: false,
    scenarios,
    ready_for_execution: scenarios.every((scenario) => scenario.status === "observed"),
  };
}

export function validateMatrixReport(report, platform, offline = false) {
  assertNoSecrets(report);
  if (report?.schema_version !== 2 || report?.benchmark !== "clark_code_consolidated") {
    throw new Error("matrix report must use clark_code_consolidated schema version 2");
  }
  if (report.status !== "passed") throw new Error("matrix report status must be passed");
  if (report.platform !== platform) {
    throw new Error(`matrix platform ${report.platform} does not match ${platform}`);
  }
  const expectedMode = offline ? "offline" : "default_paid";
  if (report.execution?.mode !== expectedMode) {
    throw new Error(`matrix mode must be ${expectedMode}, got ${report.execution?.mode}`);
  }
  if (report.execution?.paid_calls_required !== !offline) {
    throw new Error("matrix paid_calls_required does not match execution mode");
  }
  if (
    report.execution?.provider !== featureMap.live_model.provider
    || report.execution?.model !== featureMap.live_model.id
    || report.execution?.base_url !== featureMap.live_model.base_url
  ) {
    throw new Error("matrix provider, model, or endpoint drifted from the live contract");
  }
  if (report.execution?.credential_recorded !== false) {
    throw new Error("matrix must explicitly record credential_recorded=false");
  }
  const resultList = report.results || [];
  const results = new Map(resultList.map((result) => [result.id, result]));
  if (
    results.size !== Object.keys(featureMap.test_lanes).length
    || resultList.length !== results.size
  ) {
    throw new Error("matrix must contain exactly one result for every authoritative lane");
  }
  for (const [id, lane] of Object.entries(featureMap.test_lanes)) {
    const result = results.get(id);
    if (!result) throw new Error(`matrix is missing lane ${id}`);
    const supported = lane.platforms.includes(platform);
    const expectedStatus = !supported || (offline && lane.kind === "live_paid")
      ? "skipped"
      : "passed";
    if (result.status !== expectedStatus) {
      throw new Error(`${id} must be ${expectedStatus}, got ${result.status}`);
    }
  }
  const live = results.get("cheapest_paid_live_chat_jobs");
  const cost = Number(live?.reported_cost_usd || 0);
  if (!offline && !(cost > 0 && cost <= featureMap.live_model.inter_test_cost_ceiling_usd)) {
    throw new Error("paid matrix must report a positive cost within the checked-in ceiling");
  }
  return {
    status: "passed",
    mode: expectedMode,
    reported_cost_usd: cost,
    live_status: live.status,
    results,
  };
}

function scenarioResultStatus(observation, matrix, contract) {
  if (observation.status !== "observed") return observation.status;
  const deterministic = [...matrix.results.values()]
    .filter((result) => result.kind !== "live_paid" && result.status !== "skipped");
  if (deterministic.some((result) => result.status !== "passed")) return "blocked";
  if (contract.covers.includes("cheapest_paid_live_chat_and_job_round_trip")) {
    return matrix.live_status === "passed" ? "passed" : matrix.live_status;
  }
  return "passed";
}

export function buildConsolidatedReceipt(platform, observation, matrix, matrixReportPath) {
  const contracts = expectedScenarios(platform);
  const scenarios = contracts.map((contract) => {
    const observed = observation.scenarios.find((scenario) => scenario.id === contract.id);
    return {
      id: contract.id,
      covers: contract.covers,
      expected: contract.expected,
      status: scenarioResultStatus(observed, matrix, contract),
      observation: {
        status: observed.status,
        finding: observed.finding,
        assertions: observed.assertions,
        evidence: observed.evidence.map((item) => ({
          id: item.id,
          kind: item.kind,
          file: item.file || item.source_path,
          sha256: item.sha256,
          size_bytes: item.size_bytes,
        })),
      },
    };
  });
  const supported = [
    ...featureMap.features,
    ...inventory.additional_features,
  ].filter((feature) => (
    ["supported", "platform_specific"].includes(feature.platform_support[platform])
  )).map((feature) => feature.id);
  const covered = new Set(scenarios.flatMap((scenario) => scenario.covers));
  const missing = supported.filter((feature) => !covered.has(feature));
  const status = missing.length || scenarios.some((scenario) => scenario.status === "failed")
    ? "failed"
    : scenarios.every((scenario) => scenario.status === "passed")
      ? "passed"
      : "blocked";
  return {
    schema_version: 1,
    benchmark: "clark_code_platform_real_use",
    phase: "guest_execution",
    status,
    generated_at: new Date().toISOString(),
    platform,
    virtualization: platform === "macos" ? "native" : "utm",
    environment: observation.environment,
    source_revision: observation.source_revision,
    coverage: {
      supported_features: supported.length,
      covered_features: supported.length - missing.length,
      missing_features: missing,
      scenarios_required: scenarios.length,
      scenarios_passed: scenarios.filter((scenario) => scenario.status === "passed").length,
    },
    matrix: {
      status: matrix.status,
      mode: matrix.mode,
      provider: featureMap.live_model.provider,
      model: featureMap.live_model.id,
      reported_cost_usd: matrix.reported_cost_usd,
      report: matrixReportPath,
      report_sha256: matrix.report_sha256 || null,
    },
    credential_recorded: false,
    required_user_vm_actions: observation.required_user_vm_actions,
    manual_vm_actions_allowed: observation.manual_vm_actions_allowed,
    human_input_observed: observation.human_input_observed,
    scenarios,
  };
}

function exactArray(left, right) {
  return (
    Array.isArray(left)
    && left.length === right.length
    && left.every((value, index) => value === right[index])
  );
}

export function validateGuestReceipt(raw, receiptPath, requirePassed = true) {
  assertNoSecrets(raw);
  if (
    raw?.schema_version !== 1
    || raw?.benchmark !== "clark_code_platform_real_use"
    || raw?.phase !== "guest_execution"
  ) {
    throw new Error("guest receipt must use clark_code_platform_real_use schema version 1");
  }
  if (!featureMap.platforms.includes(raw.platform)) {
    throw new Error(`guest receipt has unknown platform ${raw.platform}`);
  }
  if (requirePassed && raw.status !== "passed") {
    throw new Error(`guest receipt for ${raw.platform} is ${raw.status}, not passed`);
  }
  if (raw.credential_recorded !== false) {
    throw new Error("guest receipt must explicitly record credential_recorded=false");
  }
  if (
    raw.required_user_vm_actions !== 0
    || raw.manual_vm_actions_allowed !== false
    || raw.human_input_observed !== false
  ) {
    throw new Error("guest receipt does not prove a fully autonomous execution");
  }
  if (!raw.source_revision || typeof raw.source_revision !== "string") {
    throw new Error("guest receipt requires a source_revision");
  }
  if (raw.environment?.gui_visible !== true) {
    throw new Error("passed guest receipt requires a visible GUI environment");
  }
  const expectedVm = inventory.real_use_environments[raw.platform].vm_name;
  if (expectedVm && raw.environment?.vm_name !== expectedVm) {
    throw new Error(`guest receipt must name the exact VM ${JSON.stringify(expectedVm)}`);
  }
  const expectedVirtualization = raw.platform === "macos" ? "native" : "utm";
  if (raw.virtualization !== expectedVirtualization) {
    throw new Error(`${raw.platform} guest receipt must use ${expectedVirtualization}`);
  }
  const baseDir = path.dirname(path.resolve(receiptPath));
  const matrixFile = evidenceFile(baseDir, raw.matrix?.report);
  const matrixHash = sha256File(matrixFile.canonical);
  if (matrixHash !== raw.matrix?.report_sha256) {
    throw new Error("guest matrix report SHA-256 does not match");
  }
  const offline = raw.matrix?.mode === "offline";
  if (!offline && raw.matrix?.mode !== "default_paid") {
    throw new Error(`guest matrix mode is invalid: ${raw.matrix?.mode}`);
  }
  const matrix = validateMatrixReport(
    JSON.parse(readFileSync(matrixFile.canonical, "utf8")),
    raw.platform,
    offline,
  );
  if (
    raw.matrix.status !== matrix.status
    || raw.matrix.provider !== featureMap.live_model.provider
    || raw.matrix.model !== featureMap.live_model.id
    || raw.matrix.reported_cost_usd !== matrix.reported_cost_usd
  ) {
    throw new Error("guest receipt matrix summary does not match its verified report");
  }

  const contracts = expectedScenarios(raw.platform);
  const scenarios = raw.scenarios || [];
  const scenarioIds = scenarios.map((scenario) => scenario.id);
  if (
    scenarios.length !== contracts.length
    || duplicates(scenarioIds).length
    || contracts.some((contract) => !scenarioIds.includes(contract.id))
  ) {
    throw new Error("guest receipt must contain every exact real-use scenario once");
  }
  for (const contract of contracts) {
    const scenario = scenarios.find((item) => item.id === contract.id);
    if (
      !exactArray(scenario.covers, contract.covers)
      || scenario.expected !== contract.expected
    ) {
      throw new Error(`${contract.id} drifted from its authoritative scenario contract`);
    }
    if (requirePassed && scenario.status !== "passed") {
      throw new Error(`${contract.id} is ${scenario.status}, not passed`);
    }
    if (scenario.observation?.status !== "observed") {
      throw new Error(`${contract.id} lacks a completed observation`);
    }
    if (
      !Array.isArray(scenario.observation.assertions)
      || !scenario.observation.assertions.length
      || scenario.observation.assertions.some((assertion) => assertion.status !== "passed")
    ) {
      throw new Error(`${contract.id} lacks passing observation assertions`);
    }
    if (
      !Array.isArray(scenario.observation.evidence)
      || !scenario.observation.evidence.length
    ) {
      throw new Error(`${contract.id} lacks observation evidence`);
    }
    for (const item of scenario.observation.evidence) {
      if (!ALLOWED_EVIDENCE_KINDS.has(item.kind)) {
        throw new Error(`${contract.id}/${item.id} has invalid evidence kind ${item.kind}`);
      }
      const file = evidenceFile(baseDir, item.file);
      if (
        sha256File(file.canonical) !== item.sha256
        || file.size_bytes !== item.size_bytes
      ) {
        throw new Error(`${contract.id}/${item.id} evidence integrity check failed`);
      }
    }
  }
  const supported = [
    ...featureMap.features,
    ...inventory.additional_features,
  ].filter((feature) => (
    ["supported", "platform_specific"].includes(feature.platform_support[raw.platform])
  )).map((feature) => feature.id);
  const covered = new Set(scenarios.flatMap((scenario) => scenario.covers));
  const missing = supported.filter((feature) => !covered.has(feature));
  if (
    missing.length
    || raw.coverage?.supported_features !== supported.length
    || raw.coverage?.covered_features !== supported.length
    || raw.coverage?.scenarios_required !== contracts.length
    || raw.coverage?.scenarios_passed !== scenarios.filter((item) => item.status === "passed").length
    || !exactArray(raw.coverage?.missing_features, [])
  ) {
    throw new Error("guest receipt coverage summary does not match authoritative coverage");
  }
  return {
    platform: raw.platform,
    status: raw.status,
    source_revision: raw.source_revision,
    supported_features: supported.length,
    scenarios_passed: raw.coverage.scenarios_passed,
    reported_cost_usd: matrix.reported_cost_usd,
    receipt_sha256: sha256File(receiptPath),
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

function prepareOutput(outputDir) {
  try {
    accessSync(outputDir);
    throw new Error(`refusing to overwrite real-use output ${outputDir}`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  chmodSync(outputDir, 0o700);
}

function copyEvidence(observation, outputDir) {
  const evidenceDir = path.join(outputDir, "evidence");
  mkdirSync(evidenceDir, { mode: 0o700 });
  const copied = new Map();
  for (const scenario of observation.scenarios) {
    for (const item of scenario.evidence) {
      let relative = copied.get(item.sha256);
      if (!relative) {
        const extension = path.extname(item.source_path).slice(0, 12);
        relative = path.join("evidence", `${item.sha256}${extension}`);
        const destination = path.join(outputDir, relative);
        copyFileSync(item.canonical, destination);
        secureOwnerOnlyFile(destination);
        copied.set(item.sha256, relative);
      }
      item.file = relative;
    }
  }
}

function copyGuestPackage(receipt, receiptPath, outputDir) {
  prepareOutput(outputDir);
  const baseDir = path.dirname(path.resolve(receiptPath));
  const relativeFiles = new Set([
    receipt.matrix.report,
    ...receipt.scenarios.flatMap((scenario) => (
      scenario.observation.evidence.map((item) => item.file)
    )),
  ]);
  for (const relative of relativeFiles) {
    const source = evidenceFile(baseDir, relative).canonical;
    const destination = path.join(outputDir, relative);
    mkdirSync(path.dirname(destination), { recursive: true, mode: 0o700 });
    chmodSync(path.dirname(destination), 0o700);
    copyFileSync(source, destination);
    secureOwnerOnlyFile(destination);
  }
  const destinationReceipt = path.join(outputDir, "receipt.json");
  copyFileSync(receiptPath, destinationReceipt);
  secureOwnerOnlyFile(destinationReceipt);
  return destinationReceipt;
}

function writeReceipt(outputDir, receipt) {
  const receiptPath = path.join(outputDir, "receipt.json");
  const reportPath = path.join(outputDir, "report.md");
  writeFileSync(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o600 });
  writeFileSync(
    reportPath,
    `# Clark Code ${receipt.platform} real-use benchmark

**Result:** ${receipt.status}
**Execution mode:** ${receipt.matrix.mode}
**Paid test model:** ${receipt.matrix.model}
**Reported cost:** $${receipt.matrix.reported_cost_usd}
**Feature coverage:** ${receipt.coverage.covered_features}/${receipt.coverage.supported_features}
**Scenarios passed:** ${receipt.coverage.scenarios_passed}/${receipt.coverage.scenarios_required}

| Scenario | Status |
| --- | --- |
${receipt.scenarios.map((scenario) => `| ${scenario.id} | ${scenario.status} |`).join("\n")}

Credentials are not retained. Evidence files are copied owner-only and pinned by SHA-256.
Required user VM actions: 0. Manual guest input is forbidden.
`,
    { mode: 0o600 },
  );
  secureOwnerOnlyFile(receiptPath);
  secureOwnerOnlyFile(reportPath);
  return receiptPath;
}

function blockedReceipt(platform, observation) {
  const matrix = {
    status: "not_run",
    mode: "blocked_before_paid_execution",
    reported_cost_usd: 0,
    live_status: "not_run",
    results: new Map(),
  };
  const receipt = buildConsolidatedReceipt(platform, observation, matrix, null);
  receipt.status = "blocked";
  receipt.scenarios = receipt.scenarios.map((scenario) => ({
    ...scenario,
    status: scenario.observation.status === "failed" ? "failed" : "blocked",
  }));
  receipt.coverage.scenarios_passed = 0;
  return receipt;
}

function observationTemplate(platform) {
  const vmName = inventory.real_use_environments[platform].vm_name;
  return {
    schema_version: 1,
    benchmark: "clark_code_real_use_observation",
    platform,
    generated_at: new Date().toISOString(),
    source_revision: "REPLACE_WITH_GIT_REVISION",
    credential_recorded: false,
    required_user_vm_actions: 0,
    manual_vm_actions_allowed: false,
    human_input_observed: false,
    environment: {
      gui_visible: false,
      ...(vmName ? { vm_name: vmName } : {}),
    },
    scenarios: expectedScenarios(platform).map((scenario) => ({
      id: scenario.id,
      status: "blocked",
      finding: "Replace with the current blocker, or mark observed and attach fresh evidence.",
      assertions: [],
      evidence: [],
    })),
  };
}

function runCli() {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    console.log(`Clark Code platform real-use benchmark

Usage:
  node harness/platform-real-use.mjs --observation-receipt PATH [--out PATH]
    [--matrix-receipt PATH] [--offline]
  node harness/platform-real-use.mjs --plan [--platform macos|windows|ubuntu]
  node harness/platform-real-use.mjs --write-observation-template PATH
    [--platform macos|windows|ubuntu]
  node harness/platform-real-use.mjs --verify-receipt PATH [--copy-to PATH]

The execution mode must run on the platform it claims. After complete GUI
observation evidence is validated, deterministic lanes and the cheapest-paid
tool-calling jobs run by default. --offline is the explicit no-credit mode and cannot produce
a complete real-use pass.`);
    return;
  }
  const knownFlags = new Set(["--offline", "--plan", "--help", "-h"]);
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (knownFlags.has(arg)) continue;
    if (
      [
        "--platform",
        "--out",
        "--observation-receipt",
        "--matrix-receipt",
        "--verify-receipt",
        "--copy-to",
        "--write-observation-template",
      ].includes(arg)
    ) {
      index += 1;
      continue;
    }
    if (
      [
        "--platform=",
        "--out=",
        "--observation-receipt=",
        "--matrix-receipt=",
        "--verify-receipt=",
        "--copy-to=",
        "--write-observation-template=",
      ]
        .some((prefix) => arg.startsWith(prefix))
    ) continue;
    throw new Error(`unknown argument ${JSON.stringify(arg)}`);
  }
  const verifyArg = valueArg(args, "--verify-receipt");
  if (verifyArg) {
    const sourceReceipt = path.resolve(repoDir, verifyArg);
    const raw = JSON.parse(readFileSync(sourceReceipt, "utf8"));
    let summary = validateGuestReceipt(raw, sourceReceipt);
    const copyArg = valueArg(args, "--copy-to");
    let finalReceipt = sourceReceipt;
    if (copyArg) {
      finalReceipt = copyGuestPackage(raw, sourceReceipt, path.resolve(repoDir, copyArg));
      summary = validateGuestReceipt(
        JSON.parse(readFileSync(finalReceipt, "utf8")),
        finalReceipt,
      );
    }
    console.log(JSON.stringify({ verified: true, ...summary }));
    console.log(`RECEIPT=${finalReceipt}`);
    return;
  }
  const actualPlatform = PLATFORM_BY_HOST[process.platform];
  const selectedPlatform = valueArg(args, "--platform") || actualPlatform;
  if (!featureMap.platforms.includes(selectedPlatform)) {
    throw new Error("--platform must be macos, windows, or ubuntu");
  }
  const templateArg = valueArg(args, "--write-observation-template");
  if (templateArg) {
    const templatePath = path.resolve(repoDir, templateArg);
    try {
      accessSync(templatePath);
      throw new Error(`refusing to overwrite observation template ${templatePath}`);
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    mkdirSync(path.dirname(templatePath), { recursive: true, mode: 0o700 });
    writeFileSync(
      templatePath,
      `${JSON.stringify(observationTemplate(selectedPlatform), null, 2)}\n`,
      { mode: 0o600 },
    );
    secureOwnerOnlyFile(templatePath);
    console.log(`TEMPLATE=${templatePath}`);
    return;
  }
  if (args.includes("--plan")) {
    console.log(JSON.stringify({
      platform: selectedPlatform,
      environment: inventory.real_use_environments[selectedPlatform],
      scenarios: expectedScenarios(selectedPlatform),
    }, null, 2));
    return;
  }
  const observationArg = valueArg(args, "--observation-receipt");
  if (!observationArg) throw new Error("--observation-receipt is required");
  const observationPath = path.resolve(repoDir, observationArg);
  const observation = validateObservation(
    JSON.parse(readFileSync(observationPath, "utf8")),
    selectedPlatform,
    observationPath,
  );
  const outputArg = valueArg(args, "--out");
  const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
  const outputDir = outputArg
    ? path.resolve(repoDir, outputArg)
    : path.join(repoDir, "target", "platform-real-use", `${selectedPlatform}-${stamp}-${process.pid}`);
  if (
    observation.ready_for_execution
    && (!actualPlatform || selectedPlatform !== actualPlatform)
  ) {
    throw new Error(
      `real-use execution for ${selectedPlatform} must run on that platform; host is ${process.platform}`,
    );
  }
  prepareOutput(outputDir);
  copyEvidence(observation, outputDir);
  if (!observation.ready_for_execution) {
    const receipt = blockedReceipt(selectedPlatform, observation);
    const receiptPath = writeReceipt(outputDir, receipt);
    console.log(JSON.stringify({ status: receipt.status, paid_calls_made: false }));
    console.log(`RECEIPT=${receiptPath}`);
    process.exitCode = 1;
    return;
  }

  const offline = args.includes("--offline");
  const suppliedMatrix = valueArg(args, "--matrix-receipt");
  const matrixDir = path.join(outputDir, "matrix");
  let matrixPath;
  if (suppliedMatrix) {
    mkdirSync(matrixDir, { mode: 0o700 });
    matrixPath = path.join(matrixDir, "report.json");
    copyFileSync(path.resolve(repoDir, suppliedMatrix), matrixPath);
    secureOwnerOnlyFile(matrixPath);
  } else {
    matrixPath = path.join(matrixDir, "report.json");
    const command = [
      path.join(harnessDir, "feature-matrix.mjs"),
      "--platform",
      selectedPlatform,
      "--out",
      matrixDir,
    ];
    if (offline) command.push("--offline");
    spawnSync(process.execPath, command, { cwd: repoDir, env: process.env, stdio: "inherit" });
  }
  secureOwnerOnlyFile(matrixPath);
  const matrix = validateMatrixReport(
    JSON.parse(readFileSync(matrixPath, "utf8")),
    selectedPlatform,
    offline,
  );
  matrix.report_sha256 = sha256File(matrixPath);
  const receipt = buildConsolidatedReceipt(
    selectedPlatform,
    observation,
    matrix,
    "matrix/report.json",
  );
  const receiptPath = writeReceipt(outputDir, receipt);
  console.log(JSON.stringify({
    status: receipt.status,
    scenarios_passed: receipt.coverage.scenarios_passed,
    reported_cost_usd: receipt.matrix.reported_cost_usd,
  }));
  console.log(`RECEIPT=${receiptPath}`);
  if (receipt.status !== "passed") process.exitCode = 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  runCli();
}
