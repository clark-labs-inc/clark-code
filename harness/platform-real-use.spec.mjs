import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import process from "node:process";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  buildConsolidatedReceipt,
  expectedScenarios,
  sha256File,
  validateGuestReceipt,
  validateMatrixReport,
  validateObservation,
} from "./platform-real-use.mjs";
import { validateAnyGuestReceipt } from "./platform-real-use-package.mjs";
import { isOwnerOnlyFile } from "./owner-only-file.mjs";

const environmentNames = {
  windows: "Clark QA - Windows 11 ARM",
  ubuntu: "Clark QA - Ubuntu 24.04 Desktop",
};
const hostPlatform = { darwin: "macos", win32: "windows", linux: "ubuntu" }[process.platform];
const offHostPlatform = hostPlatform === "windows" ? "ubuntu" : "windows";
const runnerPath = fileURLToPath(new URL("./platform-real-use.mjs", import.meta.url));
const packageRunnerPath = fileURLToPath(
  new URL("./platform-real-use-package.mjs", import.meta.url),
);

function fixture(platform = "windows", scenarioStatus = "observed") {
  const root = mkdtempSync(path.join(tmpdir(), "clark-platform-real-use-"));
  const evidenceDir = path.join(root, "evidence");
  mkdirSync(evidenceDir);
  const evidencePath = path.join(evidenceDir, "desktop.txt");
  writeFileSync(evidencePath, "verified desktop evidence\n");
  const hash = sha256File(evidencePath);
  const observation = {
    schema_version: 1,
    benchmark: "clark_code_real_use_observation",
    platform,
    generated_at: "2026-07-24T00:00:00Z",
    source_revision: "0123456789abcdef-dirty",
    credential_recorded: false,
    required_user_vm_actions: 0,
    manual_vm_actions_allowed: false,
    human_input_observed: false,
    environment: {
      gui_visible: scenarioStatus === "observed",
      ...(environmentNames[platform] ? { vm_name: environmentNames[platform] } : {}),
    },
    scenarios: expectedScenarios(platform).map((scenario) => ({
      id: scenario.id,
      status: scenarioStatus,
      ...(scenarioStatus === "observed"
        ? {
            assertions: [{ id: "visible_result", status: "passed", evidence: "fresh UI state" }],
            evidence: [{
              id: "desktop",
              kind: "screenshot",
              path: "evidence/desktop.txt",
              sha256: hash,
            }],
          }
        : {
            finding: "GUI is not ready",
            assertions: [],
            evidence: [],
          }),
    })),
  };
  const receiptPath = path.join(root, "observation.json");
  writeFileSync(receiptPath, `${JSON.stringify(observation)}\n`);
  return { root, observation, receiptPath };
}

function matrixReport(platform = "windows", offline = false) {
  const manifest = JSON.parse(
    // The feature map is the source of truth for lane identities in this fixture.
    readFileSync(new URL("./clark-code-feature-map.json", import.meta.url), "utf8"),
  );
  return {
    schema_version: 2,
    benchmark: "clark_code_consolidated",
    status: "passed",
    platform,
    execution: {
      mode: offline ? "offline" : "default_paid",
      paid_calls_required: !offline,
      provider: manifest.live_model.provider,
      model: manifest.live_model.id,
      base_url: manifest.live_model.base_url,
      credential_recorded: false,
    },
    results: Object.entries(manifest.test_lanes).map(([id, lane]) => ({
      id,
      kind: lane.kind,
      status: !lane.platforms.includes(platform) || (offline && lane.kind === "live_paid")
        ? "skipped"
        : "passed",
      ...(lane.kind === "live_paid" && !offline ? { reported_cost_usd: 0.12 } : {}),
    })),
  };
}

test("complete GUI evidence validates every exact Windows scenario", () => {
  const item = fixture();
  const result = validateObservation(item.observation, "windows", item.receiptPath);
  assert.equal(result.ready_for_execution, true);
  assert.equal(result.scenarios.length, expectedScenarios("windows").length);
});

test("blocked GUI evidence remains valid but cannot authorize paid execution", () => {
  const item = fixture("ubuntu", "blocked");
  const result = validateObservation(item.observation, "ubuntu", item.receiptPath);
  assert.equal(result.ready_for_execution, false);
});

test("tampered evidence is rejected before model execution", () => {
  const item = fixture();
  item.observation.scenarios[0].evidence[0].sha256 = "0".repeat(64);
  assert.throws(
    () => validateObservation(item.observation, "windows", item.receiptPath),
    /SHA-256 does not match/,
  );
});

test("credential-shaped observation values are rejected", () => {
  const item = fixture();
  item.observation.note = "Authorization: Bearer ck_live_do_not_store";
  assert.throws(
    () => validateObservation(item.observation, "windows", item.receiptPath),
    /credential-shaped/,
  );
});

test("human-assisted VM evidence is rejected before execution", () => {
  const item = fixture();
  item.observation.required_user_vm_actions = 1;
  item.observation.human_input_observed = true;
  assert.throws(
    () => validateObservation(item.observation, "windows", item.receiptPath),
    /zero required user actions/,
  );
});

test("default paid matrix requires the cheapest-paid lane and a positive bounded cost", () => {
  const result = validateMatrixReport(matrixReport(), "windows", false);
  assert.equal(result.mode, "default_paid");
  assert.equal(result.live_status, "passed");
  assert.equal(result.reported_cost_usd, 0.12);
});

test("offline matrix is explicit and cannot pass the paid real-use scenario", () => {
  const item = fixture();
  const observation = validateObservation(item.observation, "windows", item.receiptPath);
  const matrix = validateMatrixReport(matrixReport("windows", true), "windows", true);
  const receipt = buildConsolidatedReceipt(
    "windows",
    observation,
    matrix,
    "matrix/report.json",
  );
  assert.equal(receipt.status, "blocked");
  assert.equal(
    receipt.scenarios.find((scenario) => (
      scenario.covers.includes("cheapest_paid_live_chat_and_job_round_trip")
    )).status,
    "skipped",
  );
});

test("complete paid matrix and observations cover and pass every supported feature", () => {
  const item = fixture();
  const observation = validateObservation(item.observation, "windows", item.receiptPath);
  const matrix = validateMatrixReport(matrixReport(), "windows", false);
  const receipt = buildConsolidatedReceipt(
    "windows",
    observation,
    matrix,
    "matrix/report.json",
  );
  assert.equal(receipt.status, "passed");
  assert.equal(receipt.coverage.missing_features.length, 0);
  assert.equal(receipt.coverage.scenarios_passed, expectedScenarios("windows").length);
});

test("blocked CLI receipt exits before creating a matrix or making paid calls", () => {
  const item = fixture(hostPlatform, "blocked");
  const output = path.join(item.root, "blocked-output");
  const completed = spawnSync(
    process.execPath,
    [runnerPath, "--observation-receipt", item.receiptPath, "--out", output],
    { encoding: "utf8" },
  );
  assert.equal(completed.status, 1);
  assert.match(completed.stdout, /"paid_calls_made":false/);
  assert.equal(existsSync(path.join(output, "matrix")), false);
  const receipt = JSON.parse(readFileSync(path.join(output, "receipt.json"), "utf8"));
  assert.equal(receipt.status, "blocked");
  assert.equal(receipt.matrix.reported_cost_usd, 0);
});

test("blocked guest packages verify and copy without becoming a pass", () => {
  const item = fixture(hostPlatform, "blocked");
  const source = path.join(item.root, "blocked-source");
  const generated = spawnSync(
    process.execPath,
    [runnerPath, "--observation-receipt", item.receiptPath, "--out", source],
    { encoding: "utf8" },
  );
  assert.equal(generated.status, 1, generated.stderr);
  const sourceReceipt = path.join(source, "receipt.json");
  assert.equal(
    validateAnyGuestReceipt(
      JSON.parse(readFileSync(sourceReceipt, "utf8")),
      sourceReceipt,
    ).status,
    "blocked",
  );

  const copied = path.join(item.root, "blocked-copy");
  const verified = spawnSync(
    process.execPath,
    [
      packageRunnerPath,
      "--verify-receipt",
      sourceReceipt,
      "--copy-to",
      copied,
    ],
    { encoding: "utf8" },
  );
  assert.equal(verified.status, 0, verified.stderr);
  assert.match(verified.stdout, /"verified":true/);
  assert.match(verified.stdout, /"status":"blocked"/);
  const copiedReceipt = path.join(copied, "receipt.json");
  assert.equal(
    validateAnyGuestReceipt(
      JSON.parse(readFileSync(copiedReceipt, "utf8")),
      copiedReceipt,
    ).status,
    "blocked",
  );
  assert.equal(isOwnerOnlyFile(copiedReceipt), true);
});

test("off-host blocked UTM evidence produces a zero-cost receipt without execution", () => {
  const item = fixture(offHostPlatform, "blocked");
  const output = path.join(item.root, "blocked-off-host-output");
  const completed = spawnSync(
    process.execPath,
    [
      runnerPath,
      "--platform",
      offHostPlatform,
      "--observation-receipt",
      item.receiptPath,
      "--out",
      output,
    ],
    { encoding: "utf8" },
  );
  assert.equal(completed.status, 1, completed.stderr);
  assert.match(completed.stdout, /"paid_calls_made":false/);
  assert.equal(existsSync(path.join(output, "matrix")), false);
  const receipt = JSON.parse(readFileSync(path.join(output, "receipt.json"), "utf8"));
  assert.equal(receipt.platform, offHostPlatform);
  assert.equal(receipt.status, "blocked");
  assert.equal(receipt.matrix.reported_cost_usd, 0);
});

test("off-host runnable evidence is rejected before output or model execution", () => {
  const item = fixture(offHostPlatform);
  const output = path.join(item.root, "forbidden-off-host-output");
  const completed = spawnSync(
    process.execPath,
    [
      runnerPath,
      "--platform",
      offHostPlatform,
      "--observation-receipt",
      item.receiptPath,
      "--out",
      output,
    ],
    { encoding: "utf8" },
  );
  assert.notEqual(completed.status, 0);
  assert.match(completed.stderr, /must run on that platform/);
  assert.equal(existsSync(output), false);
});

test("CLI consolidates a supplied paid matrix into a self-contained pass", () => {
  const item = fixture(hostPlatform);
  const sourceMatrix = path.join(item.root, "matrix.json");
  const output = path.join(item.root, "passed-output");
  writeFileSync(sourceMatrix, `${JSON.stringify(matrixReport(hostPlatform))}\n`);
  const completed = spawnSync(
    process.execPath,
    [
      runnerPath,
      "--observation-receipt",
      item.receiptPath,
      "--matrix-receipt",
      sourceMatrix,
      "--out",
      output,
    ],
    { encoding: "utf8" },
  );
  assert.equal(completed.status, 0, completed.stderr);
  const receipt = JSON.parse(readFileSync(path.join(output, "receipt.json"), "utf8"));
  assert.equal(receipt.status, "passed");
  assert.equal(receipt.matrix.mode, "default_paid");
  assert.equal(receipt.coverage.missing_features.length, 0);
  assert.match(receipt.matrix.report_sha256, /^[a-f0-9]{64}$/);
  assert.equal(isOwnerOnlyFile(path.join(output, "receipt.json")), true);
  const verified = validateGuestReceipt(receipt, path.join(output, "receipt.json"));
  assert.equal(verified.status, "passed");

  const copied = path.join(item.root, "verified-copy");
  const verification = spawnSync(
    process.execPath,
    [runnerPath, "--verify-receipt", path.join(output, "receipt.json"), "--copy-to", copied],
    { encoding: "utf8" },
  );
  assert.equal(verification.status, 0, verification.stderr);
  assert.match(verification.stdout, /"verified":true/);
  assert.equal(
    validateGuestReceipt(
      JSON.parse(readFileSync(path.join(copied, "receipt.json"), "utf8")),
      path.join(copied, "receipt.json"),
    ).status,
    "passed",
  );
});

test("CLI writes an exact blocked observation template without any model call", () => {
  const root = mkdtempSync(path.join(tmpdir(), "clark-platform-template-"));
  const templatePath = path.join(root, "observation.json");
  const completed = spawnSync(
    process.execPath,
    [
      runnerPath,
      "--write-observation-template",
      templatePath,
      "--platform",
      "ubuntu",
    ],
    { encoding: "utf8" },
  );
  assert.equal(completed.status, 0, completed.stderr);
  const template = JSON.parse(readFileSync(templatePath, "utf8"));
  assert.equal(template.platform, "ubuntu");
  assert.equal(template.environment.vm_name, environmentNames.ubuntu);
  assert.equal(template.required_user_vm_actions, 0);
  assert.equal(template.manual_vm_actions_allowed, false);
  assert.equal(template.human_input_observed, false);
  assert.deepEqual(
    template.scenarios.map((scenario) => scenario.id),
    expectedScenarios("ubuntu").map((scenario) => scenario.id),
  );
  assert.equal(template.scenarios.every((scenario) => scenario.status === "blocked"), true);
  assert.equal(isOwnerOnlyFile(templatePath), true);
});
