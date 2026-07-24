#!/usr/bin/env node

import {
  accessSync,
  chmodSync,
  copyFileSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  realpathSync,
} from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  expectedScenarios,
  sha256File,
  validateGuestReceipt,
  validateObservation,
} from "./platform-real-use.mjs";
import { secureOwnerOnlyFile } from "./owner-only-file.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const featureMap = JSON.parse(
  readFileSync(path.join(harnessDir, "clark-code-feature-map.json"), "utf8"),
);
const inventory = JSON.parse(
  readFileSync(path.join(harnessDir, "clark-code-capability-inventory.json"), "utf8"),
);

function exactArray(left, right) {
  return (
    Array.isArray(left)
    && left.length === right.length
    && left.every((value, index) => value === right[index])
  );
}

function supportedFeatures(platform) {
  return [...featureMap.features, ...inventory.additional_features]
    .filter((feature) => (
      ["supported", "platform_specific"].includes(feature.platform_support[platform])
    ))
    .map((feature) => feature.id);
}

function preExecutionObservation(raw) {
  return {
    schema_version: 1,
    benchmark: "clark_code_real_use_observation",
    platform: raw.platform,
    generated_at: raw.generated_at,
    source_revision: raw.source_revision,
    credential_recorded: raw.credential_recorded,
    required_user_vm_actions: raw.required_user_vm_actions,
    manual_vm_actions_allowed: raw.manual_vm_actions_allowed,
    human_input_observed: raw.human_input_observed,
    environment: raw.environment,
    scenarios: raw.scenarios.map((scenario) => ({
      id: scenario.id,
      status: scenario.observation?.status,
      finding: scenario.observation?.finding,
      assertions: scenario.observation?.assertions,
      evidence: (scenario.observation?.evidence || []).map((item) => ({
        id: item.id,
        kind: item.kind,
        path: item.file,
        sha256: item.sha256,
      })),
    })),
  };
}

function validatePreExecutionBlockedReceipt(raw, receiptPath) {
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
  if (raw.status !== "blocked") {
    throw new Error(`pre-execution guest receipt must be blocked, got ${raw.status}`);
  }
  const expectedVirtualization = raw.platform === "macos" ? "native" : "utm";
  if (raw.virtualization !== expectedVirtualization) {
    throw new Error(`${raw.platform} guest receipt must use ${expectedVirtualization}`);
  }
  if (
    raw.matrix?.status !== "not_run"
    || raw.matrix?.mode !== "blocked_before_paid_execution"
    || raw.matrix?.provider !== featureMap.live_model.provider
    || raw.matrix?.model !== featureMap.live_model.id
    || raw.matrix?.reported_cost_usd !== 0
    || raw.matrix?.report !== null
    || raw.matrix?.report_sha256 !== null
  ) {
    throw new Error("blocked guest receipt must prove zero-cost pre-execution matrix state");
  }

  const contracts = expectedScenarios(raw.platform);
  const scenarios = Array.isArray(raw.scenarios) ? raw.scenarios : [];
  const ids = scenarios.map((scenario) => scenario.id);
  if (
    scenarios.length !== contracts.length
    || new Set(ids).size !== ids.length
    || contracts.some((contract) => !ids.includes(contract.id))
  ) {
    throw new Error("blocked guest receipt must contain every exact scenario once");
  }
  for (const contract of contracts) {
    const scenario = scenarios.find((item) => item.id === contract.id);
    if (
      !exactArray(scenario.covers, contract.covers)
      || scenario.expected !== contract.expected
    ) {
      throw new Error(`${contract.id} drifted from its authoritative scenario contract`);
    }
    const observationStatus = scenario.observation?.status;
    const expectedStatus = observationStatus === "failed" ? "failed" : "blocked";
    if (
      !["blocked", "failed"].includes(observationStatus)
      || scenario.status !== expectedStatus
    ) {
      throw new Error(`${contract.id} has an invalid blocked scenario state`);
    }
  }

  const observation = validateObservation(
    preExecutionObservation(raw),
    raw.platform,
    receiptPath,
  );
  if (observation.ready_for_execution) {
    throw new Error("pre-execution blocked receipt cannot authorize execution");
  }
  for (const scenario of scenarios) {
    const validated = observation.scenarios.find((item) => item.id === scenario.id);
    for (const evidence of scenario.observation.evidence || []) {
      const validatedEvidence = validated.evidence.find((item) => item.id === evidence.id);
      if (evidence.size_bytes !== validatedEvidence?.size_bytes) {
        throw new Error(`${scenario.id}/${evidence.id} evidence size does not match`);
      }
    }
  }

  const supported = supportedFeatures(raw.platform);
  if (
    raw.coverage?.supported_features !== supported.length
    || raw.coverage?.covered_features !== supported.length
    || !exactArray(raw.coverage?.missing_features, [])
    || raw.coverage?.scenarios_required !== contracts.length
    || raw.coverage?.scenarios_passed !== 0
  ) {
    throw new Error("blocked guest receipt coverage does not match authoritative coverage");
  }
  return {
    platform: raw.platform,
    status: raw.status,
    source_revision: raw.source_revision,
    supported_features: supported.length,
    scenarios_passed: 0,
    reported_cost_usd: 0,
    receipt_sha256: sha256File(receiptPath),
  };
}

export function validateAnyGuestReceipt(raw, receiptPath) {
  if (raw?.status === "passed") return validateGuestReceipt(raw, receiptPath);
  if (raw?.status !== "blocked") {
    throw new Error(`guest receipt status must be passed or blocked, got ${raw?.status}`);
  }
  if (raw.matrix?.mode === "blocked_before_paid_execution") {
    return validatePreExecutionBlockedReceipt(raw, receiptPath);
  }
  return validateGuestReceipt(raw, receiptPath, false);
}

function packageFile(baseDir, relativePath) {
  if (
    typeof relativePath !== "string"
    || !relativePath
    || path.isAbsolute(relativePath)
  ) {
    throw new Error(`package path must be a non-empty relative path: ${relativePath}`);
  }
  const base = realpathSync(baseDir);
  const candidate = path.resolve(base, relativePath);
  if (candidate !== base && !candidate.startsWith(`${base}${path.sep}`)) {
    throw new Error(`package path escapes its receipt directory: ${relativePath}`);
  }
  const metadata = lstatSync(candidate);
  if (metadata.isSymbolicLink() || !metadata.isFile()) {
    throw new Error(`package entry must be a regular non-symlink file: ${relativePath}`);
  }
  const canonical = realpathSync(candidate);
  if (canonical !== base && !canonical.startsWith(`${base}${path.sep}`)) {
    throw new Error(`package entry resolves outside its receipt directory: ${relativePath}`);
  }
  return canonical;
}

function prepareOutput(outputDir) {
  try {
    accessSync(outputDir);
    throw new Error(`refusing to overwrite real-use package output ${outputDir}`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  chmodSync(outputDir, 0o700);
}

export function copyAnyGuestPackage(raw, receiptPath, outputDir) {
  validateAnyGuestReceipt(raw, receiptPath);
  prepareOutput(outputDir);
  const baseDir = path.dirname(path.resolve(receiptPath));
  const relativeFiles = new Set([
    raw.matrix?.report,
    ...raw.scenarios.flatMap((scenario) => (
      (scenario.observation?.evidence || []).map((item) => item.file)
    )),
  ].filter((item) => typeof item === "string" && item));
  for (const relative of relativeFiles) {
    const source = packageFile(baseDir, relative);
    const destination = path.join(outputDir, relative);
    mkdirSync(path.dirname(destination), { recursive: true, mode: 0o700 });
    chmodSync(path.dirname(destination), 0o700);
    copyFileSync(source, destination);
    secureOwnerOnlyFile(destination);
  }
  const destinationReceipt = path.join(outputDir, "receipt.json");
  copyFileSync(receiptPath, destinationReceipt);
  secureOwnerOnlyFile(destinationReceipt);
  validateAnyGuestReceipt(
    JSON.parse(readFileSync(destinationReceipt, "utf8")),
    destinationReceipt,
  );
  return destinationReceipt;
}

function valueArg(args, name) {
  const index = args.indexOf(name);
  if (index < 0 || !args[index + 1] || args[index + 1].startsWith("--")) {
    throw new Error(`${name} requires a value`);
  }
  return args[index + 1];
}

function runCli() {
  const args = process.argv.slice(2);
  const receiptPath = path.resolve(repoDir, valueArg(args, "--verify-receipt"));
  const raw = JSON.parse(readFileSync(receiptPath, "utf8"));
  let summary = validateAnyGuestReceipt(raw, receiptPath);
  let finalReceipt = receiptPath;
  if (args.includes("--copy-to")) {
    const outputDir = path.resolve(repoDir, valueArg(args, "--copy-to"));
    finalReceipt = copyAnyGuestPackage(raw, receiptPath, outputDir);
    summary = validateAnyGuestReceipt(
      JSON.parse(readFileSync(finalReceipt, "utf8")),
      finalReceipt,
    );
  }
  console.log(JSON.stringify({ verified: true, ...summary }));
  console.log(`RECEIPT=${finalReceipt}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  runCli();
}
