import { spawn } from "node:child_process";
import {
  access,
  mkdir,
  readFile,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import { secureOwnerOnlyFile } from "./owner-only-file.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const manifestPath = path.join(harnessDir, "clark-code-feature-map.json");
const inventoryPath = path.join(harnessDir, "clark-code-capability-inventory.json");
const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
const inventory = JSON.parse(await readFile(inventoryPath, "utf8"));
const args = process.argv.slice(2);
const validateOnly = args.includes("--validate-only");
const offline = args.includes("--offline");
const liveOnly = args.includes("--live-only");
const wantsRealPlan = args.includes("--real-plan");
const selectedLane = valueArg("--lane");
const outputArg = valueArg("--out");
const requestedPlatform = valueArg("--platform");
const selectedPlatform = requestedPlatform || ({
  darwin: "macos",
  win32: "windows",
  linux: "ubuntu",
}[process.platform]);

if (args.includes("--help") || args.includes("-h")) {
  console.log(`Clark Code consolidated feature benchmark

Usage:
  node harness/feature-matrix.mjs [--out PATH] [--platform PLATFORM]
  node harness/feature-matrix.mjs --offline [--out PATH]
  node harness/feature-matrix.mjs --live-only [--out PATH]
  node harness/feature-matrix.mjs --validate-only
  node harness/feature-matrix.mjs --real-plan [--platform PLATFORM]
  node harness/feature-matrix.mjs --lane ID [--out PATH]

The default run executes deterministic lanes and then the cheapest-paid
tool-calling lane. --offline is the explicit no-network/no-credit mode. The paid lane reads
CLARK_CODE_API_KEY from the environment or the repository's ignored .env file;
the key is never written to output or receipts.`);
  process.exit(0);
}

const knownFlags = new Set([
  "--validate-only",
  "--offline",
  "--live-only",
  "--real-plan",
  "--help",
  "-h",
]);
for (let index = 0; index < args.length; index += 1) {
  const arg = args[index];
  if (knownFlags.has(arg)) continue;
  if (["--lane", "--out", "--platform"].includes(arg)) {
    index += 1;
    continue;
  }
  if (["--lane=", "--out=", "--platform="].some((prefix) => arg.startsWith(prefix))) continue;
  throw new Error(`unknown argument ${JSON.stringify(arg)}`);
}
if (offline && liveOnly) throw new Error("--offline and --live-only are mutually exclusive");
if (validateOnly && (offline || liveOnly || selectedLane)) {
  throw new Error("--validate-only cannot be combined with execution flags");
}
if (!manifest.platforms.includes(selectedPlatform)) {
  throw new Error(
    `unknown platform ${JSON.stringify(selectedPlatform)}; expected ${manifest.platforms.join(", ")}`,
  );
}

function valueArg(name) {
  const inline = args.find((arg) => arg.startsWith(`${name}=`));
  if (inline) return inline.slice(name.length + 1);
  const index = args.indexOf(name);
  if (index < 0) return undefined;
  const value = args[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value`);
  return value;
}

function duplicates(values) {
  const seen = new Set();
  const repeated = new Set();
  for (const value of values) {
    if (seen.has(value)) repeated.add(value);
    seen.add(value);
  }
  return [...repeated].sort();
}

function sorted(values) {
  return [...new Set(values)].sort();
}

function sameSet(left, right) {
  const a = sorted(left);
  const b = sorted(right);
  return a.length === b.length && a.every((value, index) => value === b[index]);
}

function findBalanced(source, marker, open, close) {
  const markerIndex = source.indexOf(marker);
  if (markerIndex < 0) throw new Error(`cannot find ${JSON.stringify(marker)}`);
  const start = source.indexOf(open, markerIndex + marker.length);
  if (start < 0) throw new Error(`cannot find ${open} after ${JSON.stringify(marker)}`);
  let depth = 0;
  let stringQuote = null;
  let escaped = false;
  for (let index = start; index < source.length; index += 1) {
    const char = source[index];
    if (stringQuote) {
      if (escaped) escaped = false;
      else if (char === "\\") escaped = true;
      else if (char === stringQuote) stringQuote = null;
      continue;
    }
    if (char === '"' || char === "'" || char === "`") {
      stringQuote = char;
      continue;
    }
    if (char === open) depth += 1;
    if (char === close) {
      depth -= 1;
      if (depth === 0) return source.slice(start + 1, index);
    }
  }
  throw new Error(`unterminated ${open}${close} block after ${JSON.stringify(marker)}`);
}

function rustEnumVariants(source, name) {
  const body = findBalanced(source, `enum ${name}`, "{", "}");
  return [...body.matchAll(/^\s*([A-Z][A-Za-z0-9_]*)\s*(?:,|\{|\()/gm)].map(
    (match) => match[1],
  );
}

async function extractInventoryItems(spec) {
  const sources = spec.sources || [spec.source];
  const contents = await Promise.all(
    sources.map(async (relative) => readFile(path.join(repoDir, relative), "utf8")),
  );
  const extractor = spec.extractor;
  if (extractor === "tauri_generate_handler") {
    const body = findBalanced(contents[0], "tauri::generate_handler!", "[", "]");
    return [...body.matchAll(/(?:^|\n)\s*([a-z_][a-z0-9_:]*)\s*,/g)].map(
      (match) => match[1],
    );
  }
  if (extractor === "tauri_plugins") {
    return sorted(
      [...contents[0].matchAll(/tauri_plugin_([a-z0-9_]+)::/g)].map((match) => match[1]),
    );
  }
  if (extractor === "rust_provider_impls") {
    return sorted(
      contents.flatMap((source) => (
        [...source.matchAll(/impl\s+Provider\s+for\s+([A-Za-z0-9_]+)/g)]
          .map((match) => match[1])
      )),
    );
  }
  if (extractor === "cargo_workspace_members") {
    const body = findBalanced(contents[0], "members =", "[", "]");
    return [...body.matchAll(/"([^"]+)"/g)].map((match) => match[1]);
  }
  if (extractor.startsWith("rust_trait_methods:")) {
    const name = extractor.slice("rust_trait_methods:".length);
    const body = findBalanced(contents[0], `trait ${name}`, "{", "}");
    return [...body.matchAll(/^\s*(?:async\s+)?fn\s+([a-z_][a-z0-9_]*)/gm)].map(
      (match) => match[1],
    );
  }
  if (extractor.startsWith("rust_enum_variants:")) {
    return rustEnumVariants(contents[0], extractor.slice("rust_enum_variants:".length));
  }
  if (extractor.startsWith("typescript_const_ids:")) {
    const name = extractor.slice("typescript_const_ids:".length);
    const declaration = contents[0].indexOf(name);
    if (declaration < 0) throw new Error(`cannot find TypeScript constant ${name}`);
    const assignment = contents[0].indexOf("=", declaration + name.length);
    if (assignment < 0) throw new Error(`cannot find assignment for TypeScript constant ${name}`);
    const body = findBalanced(contents[0].slice(assignment), "=", "[", "]");
    return [...body.matchAll(/\bid:\s*"([^"]+)"/g)].map((match) => match[1]);
  }
  throw new Error(`unknown inventory extractor ${JSON.stringify(extractor)}`);
}

async function validateContracts() {
  const errors = [];
  const manifestFeatureIds = manifest.features.map((feature) => feature.id);
  const additionalFeatureIds = inventory.additional_features.map((feature) => feature.id);
  const allFeatures = [...manifest.features, ...inventory.additional_features];
  const allFeatureIds = [...manifestFeatureIds, ...additionalFeatureIds];
  const featureSet = new Set(allFeatureIds);
  const laneSet = new Set(Object.keys(manifest.test_lanes));

  const duplicateFeatures = duplicates(allFeatureIds);
  if (duplicateFeatures.length) errors.push(`duplicate feature ids: ${duplicateFeatures.join(", ")}`);

  const declaredTools = new Set(manifest.model_tools);
  const mappedTools = manifest.features.flatMap((feature) => feature.tools);
  const duplicateToolMappings = duplicates(mappedTools);
  const missingTools = [...declaredTools].filter((tool) => !mappedTools.includes(tool)).sort();
  const unknownTools = mappedTools.filter((tool) => !declaredTools.has(tool)).sort();
  if (duplicateToolMappings.length) {
    errors.push(`model tools mapped more than once: ${duplicateToolMappings.join(", ")}`);
  }
  if (missingTools.length) errors.push(`declared model tools without a feature: ${missingTools.join(", ")}`);
  if (unknownTools.length) errors.push(`feature tools absent from model_tools: ${unknownTools.join(", ")}`);

  if (manifest.live_model.id !== "clark-code:minimax_m3") {
    errors.push(`default live model must be clark-code:minimax_m3, got ${manifest.live_model.id}`);
  }
  if (manifest.live_model.default_paid !== true) {
    errors.push("live_model.default_paid must be true");
  }
  if (
    manifest.live_model.selection_policy
    !== "lowest_expected_cost_for_input_heavy_tool_calling_tests"
  ) {
    errors.push("live_model.selection_policy must pin the input-heavy cheapest-paid policy");
  }
  if (manifest.live_model.upstream_id !== "minimax/minimax-m3") {
    errors.push(`live_model.upstream_id must be minimax/minimax-m3, got ${manifest.live_model.upstream_id}`);
  }
  if (manifest.live_model.temperature !== 0) {
    errors.push("live_model.temperature must be 0 for reproducible paid benchmark turns");
  }
  if (manifest.test_lanes.cheapest_paid_live_chat_jobs?.kind !== "live_paid") {
    errors.push("cheapest_paid_live_chat_jobs must exist and be a live_paid lane");
  }

  for (const feature of allFeatures) {
    for (const platform of manifest.platforms) {
      const support = feature.platform_support[platform];
      if (!manifest.support_states.includes(support)) {
        errors.push(`${feature.id}: invalid ${platform} support state ${JSON.stringify(support)}`);
      }
    }
    for (const lane of feature.lanes) {
      if (!laneSet.has(lane)) errors.push(`${feature.id}: unknown test lane ${lane}`);
    }
  }

  const inventoryCounts = {};
  for (const [inventoryId, spec] of Object.entries(inventory.inventories)) {
    const declared = spec.groups.flatMap((group) => group.items);
    const actual = await extractInventoryItems(spec);
    inventoryCounts[inventoryId] = actual.length;
    if (inventoryId === "coding_models" && !actual.includes(manifest.live_model.id)) {
      errors.push(
        `coding_models: default live model is absent from the product picker: ${manifest.live_model.id}`,
      );
    }
    const repeated = duplicates(declared);
    if (repeated.length) {
      errors.push(`${inventoryId}: items mapped more than once: ${repeated.join(", ")}`);
    }
    if (!sameSet(declared, actual)) {
      const declaredSet = new Set(declared);
      const actualSet = new Set(actual);
      const missing = actual.filter((item) => !declaredSet.has(item));
      const stale = declared.filter((item) => !actualSet.has(item));
      if (missing.length) errors.push(`${inventoryId}: unmapped code items: ${missing.join(", ")}`);
      if (stale.length) errors.push(`${inventoryId}: stale declared items: ${stale.join(", ")}`);
    }
    for (const group of spec.groups) {
      for (const feature of group.features) {
        if (!featureSet.has(feature)) errors.push(`${inventoryId}/${group.id}: unknown feature ${feature}`);
      }
      for (const lane of group.lanes) {
        if (!laneSet.has(lane)) errors.push(`${inventoryId}/${group.id}: unknown lane ${lane}`);
      }
    }
  }

  const controlIds = inventory.security_controls.map((control) => control.id);
  const duplicateControls = duplicates(controlIds);
  if (duplicateControls.length) {
    errors.push(`duplicate security control ids: ${duplicateControls.join(", ")}`);
  }
  for (const control of inventory.security_controls) {
    if (!featureSet.has(control.feature)) {
      errors.push(`${control.id}: unknown feature ${control.feature}`);
    }
    if (!laneSet.has(control.lane)) errors.push(`${control.id}: unknown lane ${control.lane}`);
    const evidencePath = path.join(repoDir, control.evidence.path);
    try {
      const source = await readFile(evidencePath, "utf8");
      if (!source.includes(control.evidence.contains)) {
        errors.push(
          `${control.id}: evidence marker absent from ${control.evidence.path}: ${JSON.stringify(control.evidence.contains)}`,
        );
      }
    } catch (error) {
      errors.push(`${control.id}: cannot read ${control.evidence.path}: ${error.message}`);
    }
  }

  for (const platform of manifest.platforms) {
    const baseCovered = new Set(
      manifest.real_use_scenarios[platform].flatMap((scenario) => scenario.covers),
    );
    const baseExpected = manifest.features
      .filter((feature) => ["supported", "platform_specific"].includes(feature.platform_support[platform]))
      .map((feature) => feature.id);
    const baseMissing = baseExpected.filter((feature) => !baseCovered.has(feature));
    if (baseMissing.length) {
      errors.push(`${platform}: no base real-use scenario covers ${baseMissing.join(", ")}`);
    }

    const additionalCovered = new Set(
      inventory.real_use_scenarios[platform].flatMap((scenario) => scenario.covers),
    );
    const additionalExpected = inventory.additional_features
      .filter((feature) => ["supported", "platform_specific"].includes(feature.platform_support[platform]))
      .map((feature) => feature.id);
    const additionalMissing = additionalExpected.filter((feature) => !additionalCovered.has(feature));
    if (additionalMissing.length) {
      errors.push(`${platform}: no extended real-use scenario covers ${additionalMissing.join(", ")}`);
    }
  }

  const windows = inventory.real_use_environments.windows;
  const ubuntu = inventory.real_use_environments.ubuntu;
  if (windows.virtualization !== "utm" || ubuntu.virtualization !== "utm") {
    errors.push("Windows and Ubuntu real-use environments must use UTM");
  }
  if (
    typeof windows.vm_name !== "string"
    || typeof ubuntu.vm_name !== "string"
    || !windows.vm_name
    || !ubuntu.vm_name
    || windows.vm_name === ubuntu.vm_name
    || [windows.vm_name, ubuntu.vm_name].some((name) => name.includes("/") || name.includes("\\"))
  ) {
    errors.push("Windows and Ubuntu must declare distinct, path-safe UTM vm_name values");
  }
  if (!windows.gui_required || !ubuntu.gui_required || !ubuntu.desktop_required) {
    errors.push("Windows and Ubuntu must require GUI environments, and Ubuntu must require Desktop");
  }
  const realRunner = inventory.real_use_runner;
  const guestProductCommands = realRunner?.guest_product_commands;
  const guestQaAuth = realRunner?.guest_qa_auth;
  if (
    realRunner?.host_platform !== "macos"
    || realRunner?.phase !== "environment_preflight"
    || realRunner?.ready_status !== "ready"
    || realRunner?.blocked_status !== "blocked"
    || realRunner?.command?.join(" ") !== "node harness/utm-real-use.mjs"
  ) {
    errors.push("real_use_runner must pin the macOS UTM environment-preflight contract");
  }
  if (
    !sameSet(realRunner?.guest_platforms || [], manifest.platforms)
    || realRunner?.guest_command?.join(" ") !== "node harness/platform-real-use.mjs"
    || realRunner?.guest_package_verifier_command?.join(" ")
      !== "node harness/platform-real-use-package.mjs"
    || realRunner?.guest_phase !== "guest_execution"
    || realRunner?.guest_receipt_schema_version !== 1
    || realRunner?.guest_default_paid !== true
    || realRunner?.guest_offline_flag !== "--offline"
    || realRunner?.evidence_integrity !== "sha256"
    || realRunner?.guest_source_stage_command?.join(" ")
      !== "node harness/utm-source-stage.mjs stage --platform all"
    || realRunner?.guest_provision_command?.join(" ")
      !== "node harness/utm-guest-provision.mjs ensure --platform all"
    || realRunner?.guest_deterministic_command?.join(" ")
      !== "node harness/utm-guest-benchmark.mjs run --offline --platform all"
    || guestProductCommands?.windows?.join(" ")
      !== "node harness/utm-windows-journey.mjs auth-smoke"
    || guestProductCommands?.ubuntu?.join(" ")
      !== "node harness/utm-ubuntu-journey.mjs auth-smoke"
    || guestQaAuth?.credential_source !== ".env"
    || guestQaAuth?.required_email_domain !== "clarkslabs.com"
    || guestQaAuth?.session_kind !== "short_lived_jwt"
    || guestQaAuth?.provider_key_binding !== "same_account"
    || guestQaAuth?.transient_secret_transfer_erased !== true
    || guestQaAuth?.credential_values_in_receipts !== false
    || realRunner?.guest_deterministic_integrity !== "sha256"
    || realRunner?.guest_long_job_transport !== "detached_authenticated_file_channel"
    || realRunner?.guest_source_includes_dirty_worktree !== true
    || realRunner?.guest_source_ignored_env_included !== false
    || realRunner?.guest_source_integrity !== "sha256"
    || realRunner?.consolidated_command?.join(" ")
      !== "bash scripts/run-pre-release-benchmarks.sh"
    || realRunner?.guest_receipt_flag !== "--real-use-receipt"
    || realRunner?.complete_platform_set_required !== true
    || realRunner?.blocked_package_status_preserved !== true
  ) {
    errors.push("real_use_runner must pin the cross-platform paid guest-evidence contract");
  }
  const vmAutonomy = realRunner?.vm_autonomy;
  if (
    vmAutonomy?.required_user_vm_actions !== 0
    || vmAutonomy?.manual_vm_actions_allowed !== false
    || vmAutonomy?.release_requires_human_vm_action !== false
    || vmAutonomy?.virtualization !== "utm"
    || !sameSet(vmAutonomy?.forbidden_virtualization || [], ["parallels"])
    || vmAutonomy?.command?.join(" ")
      !== "node harness/utm-autonomy.mjs ensure --platform all"
    || vmAutonomy?.audit_command?.join(" ")
      !== "node harness/utm-autonomy.mjs audit --platform all"
    || vmAutonomy?.observation_command?.join(" ")
      !== "node harness/utm-window-observation.mjs --platform all"
    || vmAutonomy?.credential_source !== ".env"
    || !sameSet(
      vmAutonomy?.credential_keys || [],
      ["CLARK_QA_VM_USERNAME", "CLARK_QA_VM_PASSWORD"],
    )
    || vmAutonomy?.credential_values_in_receipts !== false
    || vmAutonomy?.guest_transport !== "qemu_guest_agent_file_channel"
    || vmAutonomy?.guest_probe_authentication !== "per_run_random_marker"
    || vmAutonomy?.qmp_bind !== "127.0.0.1"
    || vmAutonomy?.physical_input !== "optional_non_release_diagnostic_only"
  ) {
    errors.push("real_use_runner must require a zero-user-action UTM lifecycle");
  }
  for (const [platform, environment] of Object.entries({ windows, ubuntu })) {
    if (
      !Number.isInteger(environment.autonomy?.qmp_port)
      || environment.autonomy.qmp_port < 1024
      || environment.autonomy.required_user_vm_actions !== 0
      || environment.autonomy.recovery_input !== "localhost_qmp"
    ) {
      errors.push(`${platform}: UTM autonomy configuration is incomplete`);
    }
  }
  if (windows.autonomy.qmp_port === ubuntu.autonomy.qmp_port) {
    errors.push("Windows and Ubuntu must use distinct localhost QMP ports");
  }
  const utmLane = manifest.test_lanes.utm_harness_contract;
  if (
    utmLane?.kind !== "simulated"
    || !sameSet(utmLane?.platforms || [], ["macos"])
    || utmLane?.steps?.length !== 1
    || utmLane.steps[0].join(" ") !== "node --test harness/utm-real-use.spec.mjs"
  ) {
    errors.push("utm_harness_contract must test the checked-in UTM runner on macOS");
  }
  const platformRealUseLane = manifest.test_lanes.platform_real_use_contract;
  if (
    platformRealUseLane?.kind !== "simulated"
    || !sameSet(platformRealUseLane?.platforms || [], manifest.platforms)
    || platformRealUseLane?.steps?.length !== 1
    || platformRealUseLane.steps[0].join(" ")
      !== "node --test harness/platform-real-use.spec.mjs"
  ) {
    errors.push("platform_real_use_contract must test the guest evidence runner everywhere");
  }

  if (errors.length) throw new Error(`feature and capability contracts are invalid:\n- ${errors.join("\n- ")}`);
  return {
    schema_version: manifest.schema_version,
    features: allFeatures.length,
    base_features: manifest.features.length,
    extended_features: inventory.additional_features.length,
    model_tools: manifest.model_tools.length,
    native_commands: inventoryCounts.native_commands,
    security_controls: inventory.security_controls.length,
    test_lanes: Object.keys(manifest.test_lanes).length,
    real_scenarios:
      Object.values(manifest.real_use_scenarios).flat().length
      + Object.values(inventory.real_use_scenarios).flat().length,
    inventories: inventoryCounts,
  };
}

async function loadBenchmarkEnv() {
  const accepted = new Set([
    "CLARK_CODE_API_KEY",
    "CLARK_CODE_PROVIDER",
    "CLARK_CODE_BASE_URL",
    "CLARK_CODE_MODEL",
  ]);
  try {
    const source = await readFile(path.join(repoDir, ".env"), "utf8");
    for (const rawLine of source.split(/\r?\n/)) {
      const line = rawLine.trim();
      if (!line || line.startsWith("#")) continue;
      const separator = line.indexOf("=");
      if (separator < 1) continue;
      const name = line.slice(0, separator).trim();
      if (!accepted.has(name) || process.env[name]) continue;
      let value = line.slice(separator + 1).trim();
      if (
        (value.startsWith('"') && value.endsWith('"'))
        || (value.startsWith("'") && value.endsWith("'"))
      ) {
        value = value.slice(1, -1);
      }
      if (value) process.env[name] = value;
    }
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  process.env.CLARK_CODE_PROVIDER ||= manifest.live_model.provider;
  process.env.CLARK_CODE_BASE_URL ||= manifest.live_model.base_url;
  process.env.CLARK_CODE_MODEL ||= manifest.live_model.id;
}

function livePreflight() {
  const errors = [];
  if (!process.env.CLARK_CODE_API_KEY?.trim()) errors.push("CLARK_CODE_API_KEY is missing");
  if (process.env.CLARK_CODE_PROVIDER !== manifest.live_model.provider) {
    errors.push(`CLARK_CODE_PROVIDER must be ${manifest.live_model.provider}`);
  }
  if (process.env.CLARK_CODE_MODEL !== manifest.live_model.id) {
    errors.push(
      `CLARK_CODE_MODEL must be ${manifest.live_model.id} for the default cheapest-paid control`,
    );
  }
  if (process.env.CLARK_CODE_BASE_URL !== manifest.live_model.base_url) {
    errors.push(`CLARK_CODE_BASE_URL must be ${manifest.live_model.base_url}`);
  }
  return errors;
}

const secretValues = [];
function redact(text) {
  let safe = String(text)
    .replace(/\bck_(?:live|test)_[A-Za-z0-9._-]+\b/g, "ck_[REDACTED]")
    .replace(/(authorization["']?\s*[:=]\s*["']?bearer\s+)[^\s"',}]+/gi, "$1[REDACTED]");
  for (const secret of secretValues) {
    if (secret) safe = safe.split(secret).join("[REDACTED]");
  }
  return safe.slice(-40_000);
}

function windowsCommand(command) {
  if (process.platform !== "win32" || command[0] !== "corepack") {
    return { executable: command[0], args: command.slice(1) };
  }
  const safe = /^[A-Za-z0-9_@./:\\=+-]+$/;
  if (command.some((part) => !safe.test(part))) {
    throw new Error("Windows command contains a shell-unsafe manifest argument");
  }
  return {
    executable: process.env.ComSpec || "C:\\Windows\\System32\\cmd.exe",
    args: ["/d", "/s", "/c", ["corepack.cmd", ...command.slice(1)].join(" ")],
  };
}

function runStep(command, lane, logPath) {
  return new Promise((resolve) => {
    const startedAt = Date.now();
    const env = { ...process.env };
    if (lane.kind === "live_paid") {
      env.CLARK_CODE_LIVE = "1";
      env.CLARK_CODE_MAX_ITERATIONS = String(manifest.live_model.max_iterations_per_turn);
    }
    if (selectedPlatform === "macos" && lane.macos_dyld_swift_runtime) {
      env.DYLD_LIBRARY_PATH = [
        "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift-5.5/macosx",
        env.DYLD_LIBRARY_PATH,
      ].filter(Boolean).join(":");
    }
    process.stdout.write(`\n$ ${command.join(" ")}\n`);
    let output = "";
    const collect = (chunk) => {
      const safe = redact(chunk.toString());
      output = redact(output + safe);
      process.stdout.write(safe);
    };
    let child;
    try {
      const invocation = windowsCommand(command);
      child = spawn(invocation.executable, invocation.args, {
        cwd: repoDir,
        env,
        stdio: ["ignore", "pipe", "pipe"],
      });
    } catch (error) {
      const result = {
        command,
        status: "failed",
        duration_ms: Date.now() - startedAt,
        exit_code: null,
        output_tail: redact(`${output}\n${error}`),
      };
      writeFile(logPath, `${result.output_tail}\n`, { mode: 0o600 }).then(() => {
        secureOwnerOnlyFile(logPath);
        resolve(result);
      });
      return;
    }
    child.stdout.on("data", collect);
    child.stderr.on("data", collect);
    child.once("error", async (error) => {
      const result = {
        command,
        status: "failed",
        duration_ms: Date.now() - startedAt,
        exit_code: null,
        output_tail: redact(`${output}\n${error}`),
      };
      await writeFile(logPath, `${result.output_tail}\n`, { mode: 0o600 });
      secureOwnerOnlyFile(logPath);
      resolve(result);
    });
    child.once("exit", async (code, signal) => {
      const result = {
        command,
        status: code === 0 ? "passed" : "failed",
        duration_ms: Date.now() - startedAt,
        exit_code: code,
        signal,
        output_tail: output,
      };
      await writeFile(logPath, `${output}\n`, { mode: 0o600 });
      secureOwnerOnlyFile(logPath);
      resolve(result);
    });
  });
}

function reportedCost(output) {
  return [...output.matchAll(/cost_usd:\s*Some\(([0-9]+(?:\.[0-9]+)?)\)/g)]
    .reduce((sum, match) => sum + Number(match[1]), 0);
}

async function runLane(id, lane, artifactDir) {
  const startedAt = Date.now();
  if (!lane.platforms.includes(selectedPlatform)) {
    return { id, kind: lane.kind, status: "skipped", reason: `not defined for ${selectedPlatform}`, steps: [] };
  }
  const steps = [];
  let liveCostUsd = 0;
  for (let index = 0; index < lane.steps.length; index += 1) {
    if (
      lane.kind === "live_paid"
      && liveCostUsd >= manifest.live_model.inter_test_cost_ceiling_usd
    ) {
      return {
        id,
        kind: lane.kind,
        status: "budget_exhausted",
        reason: `reported inter-test cost reached $${manifest.live_model.inter_test_cost_ceiling_usd}`,
        reported_cost_usd: liveCostUsd,
        duration_ms: Date.now() - startedAt,
        steps,
      };
    }
    const logPath = path.join(artifactDir, `${id}-${index + 1}.log`);
    const result = await runStep(lane.steps[index], lane, logPath);
    steps.push({ ...result, log: path.relative(artifactDir, logPath) });
    liveCostUsd += reportedCost(result.output_tail);
    if (result.status !== "passed") break;
  }
  return {
    id,
    kind: lane.kind,
    status: steps.length === lane.steps.length && steps.every((step) => step.status === "passed")
      ? "passed"
      : "failed",
    ...(lane.kind === "live_paid" ? { reported_cost_usd: liveCostUsd } : {}),
    duration_ms: Date.now() - startedAt,
    steps,
  };
}

function artifactDirectory() {
  if (outputArg) return path.resolve(repoDir, outputArg);
  const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
  return path.join(repoDir, "target", "clark-code-benchmark", `${stamp}-${process.pid}`);
}

const validation = await validateContracts();
console.log(JSON.stringify({ contracts: "valid", platform: selectedPlatform, ...validation }));

if (wantsRealPlan) {
  console.log(JSON.stringify({
    platform: selectedPlatform,
    environment: inventory.real_use_environments[selectedPlatform],
    scenarios: [
      ...manifest.real_use_scenarios[selectedPlatform],
      ...inventory.real_use_scenarios[selectedPlatform],
    ],
  }, null, 2));
}

if (!validateOnly && !wantsRealPlan) {
  await loadBenchmarkEnv();
  if (process.env.CLARK_CODE_API_KEY) secretValues.push(process.env.CLARK_CODE_API_KEY);
  const artifactDir = artifactDirectory();
  try {
    await access(artifactDir);
    throw new Error(`refusing to overwrite benchmark output ${artifactDir}`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  await mkdir(artifactDir, { recursive: true, mode: 0o700 });
  const allEntries = Object.entries(manifest.test_lanes);
  let entries;
  if (selectedLane) {
    entries = allEntries.filter(([id]) => id === selectedLane);
    if (entries.length === 0) throw new Error(`unknown lane ${JSON.stringify(selectedLane)}`);
  } else if (liveOnly) {
    entries = allEntries.filter(([, lane]) => lane.kind === "live_paid");
  } else if (offline) {
    entries = allEntries.filter(([, lane]) => lane.kind !== "live_paid");
  } else {
    entries = allEntries;
  }

  const deterministicEntries = entries.filter(([, lane]) => lane.kind !== "live_paid");
  const liveEntries = entries.filter(([, lane]) => lane.kind === "live_paid");
  const results = [];
  for (const [id, lane] of deterministicEntries) {
    results.push(await runLane(id, lane, artifactDir));
  }
  const deterministicFailed = results.some((result) => result.status === "failed");
  const preflightErrors = liveEntries.length ? livePreflight() : [];
  for (const [id, lane] of liveEntries) {
    if (deterministicFailed && !liveOnly) {
      results.push({
        id,
        kind: lane.kind,
        status: "blocked",
        reason: "deterministic contract failed; paid calls were not made",
        steps: [],
      });
    } else if (preflightErrors.length) {
      results.push({
        id,
        kind: lane.kind,
        status: "configuration_failed",
        reason: preflightErrors.join("; "),
        steps: [],
      });
    } else {
      results.push(await runLane(id, lane, artifactDir));
    }
  }
  if (offline && !selectedLane) {
    for (const [id, lane] of allEntries.filter(([, lane]) => lane.kind === "live_paid")) {
      results.push({
        id,
        kind: lane.kind,
        status: "skipped",
        reason: "explicit --offline opt-out",
        steps: [],
      });
    }
  }

  const failingStates = new Set(["failed", "blocked", "configuration_failed", "budget_exhausted"]);
  const report = {
    schema_version: 2,
    benchmark: "clark_code_consolidated",
    status: results.some((result) => failingStates.has(result.status)) ? "failed" : "passed",
    manifest: path.relative(repoDir, manifestPath),
    inventory: path.relative(repoDir, inventoryPath),
    platform: selectedPlatform,
    started_at: new Date().toISOString(),
    validation,
    execution: {
      mode: offline ? "offline" : liveOnly ? "live_only" : selectedLane ? "selected_lane" : "default_paid",
      paid_calls_required: !offline && liveEntries.length > 0,
      provider: manifest.live_model.provider,
      model: manifest.live_model.id,
      base_url: manifest.live_model.base_url,
      credential_recorded: false,
      max_live_tests: manifest.live_model.max_live_tests,
      max_iterations_per_turn: manifest.live_model.max_iterations_per_turn,
      temperature: manifest.live_model.temperature,
      inter_test_cost_ceiling_usd: manifest.live_model.inter_test_cost_ceiling_usd,
    },
    environment: inventory.real_use_environments[selectedPlatform],
    real_use_scenarios: [
      ...manifest.real_use_scenarios[selectedPlatform],
      ...inventory.real_use_scenarios[selectedPlatform],
    ],
    security_controls: inventory.security_controls.map(({ id, feature, lane, evidence }) => ({
      id,
      feature,
      lane,
      evidence_path: evidence.path,
      evidence_marker_present: true,
    })),
    results,
    summary: {
      passed: results.filter((result) => result.status === "passed").length,
      failed: results.filter((result) => result.status === "failed").length,
      blocked: results.filter((result) => result.status === "blocked").length,
      configuration_failed: results.filter((result) => result.status === "configuration_failed").length,
      budget_exhausted: results.filter((result) => result.status === "budget_exhausted").length,
      skipped: results.filter((result) => result.status === "skipped").length,
    },
  };
  const reportPath = path.join(artifactDir, "report.json");
  const markdownPath = path.join(artifactDir, "report.md");
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, { mode: 0o600 });
  await writeFile(
    markdownPath,
    `# Clark Code consolidated benchmark

**Result:** ${report.status}
**Platform:** ${selectedPlatform}
**Mode:** ${report.execution.mode}
**Default live model:** ${report.execution.model}
**Paid calls required:** ${report.execution.paid_calls_required}
**Features mapped:** ${validation.features}
**Model tools mapped:** ${validation.model_tools}
**Native commands mapped:** ${validation.native_commands}
**Security controls mapped:** ${validation.security_controls}

| Lane | Kind | Status |
| --- | --- | --- |
${results.map((result) => `| ${result.id} | ${result.kind} | ${result.status} |`).join("\n")}

The API key is never written to benchmark artifacts. A deterministic pass is
not reported as a live pass; live configuration failures, blocks, and skips
remain distinct states.
`,
    { mode: 0o600 },
  );
  secureOwnerOnlyFile(reportPath);
  secureOwnerOnlyFile(markdownPath);
  console.log(JSON.stringify(report.summary));
  console.log(`REPORT=${reportPath}`);
  if (report.status !== "passed") process.exitCode = 1;
}
