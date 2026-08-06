import { spawnSync } from "node:child_process";
import { access, mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const contractPath = path.join(harnessDir, "clark-tui-product.contract.json");

function duplicates(values) {
  const seen = new Set();
  const repeated = new Set();
  for (const value of values) {
    if (seen.has(value)) repeated.add(value);
    seen.add(value);
  }
  return [...repeated].sort();
}

async function exists(pathname) {
  try {
    await access(pathname);
    return true;
  } catch {
    return false;
  }
}

async function sourceFiles(relativePath) {
  const pathname = path.join(repoDir, relativePath);
  if (!(await exists(pathname))) return [];
  const entries = await readdir(pathname, { withFileTypes: true }).catch(() => null);
  if (!entries) return [pathname];
  const nested = await Promise.all(entries.map((entry) => {
    const child = path.join(relativePath, entry.name);
    return entry.isDirectory() ? sourceFiles(child) : [path.join(repoDir, child)];
  }));
  return nested.flat();
}

async function verifyImplementationBoundary(boundary) {
  const files = await sourceFiles(boundary.source_root);
  const manifest = await readFile(path.join(repoDir, boundary.manifest), "utf8");
  const dependencies = manifest.match(/\[dependencies\]([\s\S]*?)(?=\n\[|$)/)?.[1] ?? "";
  const names = [...dependencies.matchAll(/^([A-Za-z0-9_-]+)\s*=/gm)].map((match) => match[1]);
  const terminalCrates = names.filter((name) => /tui|terminal|crossterm/i.test(name));
  const unexpected = terminalCrates.filter((name) => !boundary.allowed_terminal_crates.includes(name));
  if (unexpected.length) {
    throw new Error(`Clark-owned TUI boundary has unapproved terminal application dependencies: ${unexpected.join(", ")}`);
  }
  const forbidden = boundary.forbidden_source_terms ?? [];
  for (const pathname of [path.join(repoDir, boundary.manifest), ...files]) {
    const source = await readFile(pathname, "utf8");
    const lower = source.toLowerCase();
    const term = forbidden.find((candidate) => lower.includes(candidate.toLowerCase()));
    if (term) {
      throw new Error(`Clark-owned TUI boundary found forbidden external implementation term ${JSON.stringify(term)} in ${path.relative(repoDir, pathname)}`);
    }
  }
  return {
    scanned_file_count: files.length,
    terminal_crates: terminalCrates.sort(),
    forbidden_source_terms: forbidden,
  };
}

function runStep(step) {
  const result = spawnSync(step.program, step.args ?? [], {
    cwd: repoDir,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  });
  return {
    passed: result.status === 0,
    detail: result.status === 0
      ? `${step.program} ${step.args?.join(" ") ?? ""} passed`
      : `${step.program} failed (${result.status ?? "no status"}): ${(result.stderr || result.stdout || result.error?.message || "no output").trim()}`,
  };
}

async function runProbe(probe, commandProbeCache) {
  if (probe.kind === "commands") {
    const cacheKey = JSON.stringify(probe.steps);
    if (commandProbeCache.has(cacheKey)) return commandProbeCache.get(cacheKey);
    await mkdir(path.join(repoDir, "target", "clark-tui-product"), { recursive: true });
    for (const step of probe.steps) {
      const result = runStep(step);
      if (!result.passed) {
        commandProbeCache.set(cacheKey, result);
        return result;
      }
    }
    const result = { passed: true, detail: `${probe.steps.length} behavior-test commands passed` };
    commandProbeCache.set(cacheKey, result);
    return result;
  }
  const pathname = path.join(repoDir, probe.path);
  if (!(await exists(pathname))) {
    return { passed: false, detail: `${probe.path} does not exist` };
  }
  const source = await readFile(pathname, "utf8");
  if (probe.kind === "file_contains" && !source.includes(probe.contains)) {
    return { passed: false, detail: `${probe.path} does not contain ${JSON.stringify(probe.contains)}` };
  }
  if (probe.kind !== "file_contains") throw new Error(`unknown probe kind ${JSON.stringify(probe.kind)}`);
  return { passed: true, detail: `${probe.path} contains required Clark integration evidence` };
}

export async function evaluateContract() {
  const contract = JSON.parse(await readFile(contractPath, "utf8"));
  if (contract.schema_version !== 3) throw new Error("unsupported Clark TUI product-contract schema");
  const ids = contract.features.map((feature) => feature.id);
  const duplicateIds = duplicates(ids);
  if (duplicateIds.length) throw new Error(`duplicate feature ids: ${duplicateIds.join(", ")}`);
  const commands = contract.features.flatMap((feature) => feature.commands);
  const duplicateCommands = duplicates(commands);
  if (duplicateCommands.length) throw new Error(`commands assigned to multiple features: ${duplicateCommands.join(", ")}`);
  const behaviorIds = contract.features.flatMap((feature) => feature.behaviors.map((_, index) => `${feature.id}:${index}`));
  if (behaviorIds.length === 0 || contract.features.some((feature) => feature.behaviors.length === 0)) {
    throw new Error("every Clark TUI feature needs at least one behavior requirement");
  }

  const boundary = await verifyImplementationBoundary(contract.implementation_boundary);
  const features = [];
  const commandProbeCache = new Map();
  for (const feature of contract.features) {
    if (!["implemented", "gap"].includes(feature.expected_state)) throw new Error(`invalid expected_state for ${feature.id}`);
    const probes = [];
    for (const probe of feature.probes) probes.push(await runProbe(probe, commandProbeCache));
    const actualState = probes.every((probe) => probe.passed) ? "implemented" : "gap";
    if (actualState !== feature.expected_state) {
      throw new Error(`${feature.id} changed from expected ${feature.expected_state} to ${actualState}; update behavior tests and the contract together`);
    }
    features.push({
      id: feature.id,
      title: feature.title,
      state: actualState,
      commands: feature.commands,
      behaviors: feature.behaviors,
      first_failure: probes.find((probe) => !probe.passed)?.detail ?? null,
    });
  }

  const gapFeatures = features.filter((feature) => feature.state === "gap");
  return {
    receipt_type: "clark_tui_product_contract",
    schema_version: contract.schema_version,
    evidence_class: contract.evidence_class,
    implementation_boundary: { ...contract.implementation_boundary, ...boundary },
    summary: {
      feature_count: features.length,
      implemented_count: features.length - gapFeatures.length,
      gap_count: gapFeatures.length,
      behavior_count: behaviorIds.length,
      command_count: commands.length,
      command_gaps: gapFeatures.flatMap((feature) => feature.commands).length,
      complete: gapFeatures.length === 0,
    },
    features,
  };
}

async function main() {
  const args = process.argv.slice(2);
  const known = new Set(["--require-complete", "--json", "--help", "-h"]);
  let output;
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (known.has(arg)) continue;
    if (arg === "--out") {
      output = args[index + 1];
      if (!output || output.startsWith("--")) throw new Error("--out requires a path");
      index += 1;
      continue;
    }
    if (arg.startsWith("--out=")) {
      output = arg.slice("--out=".length);
      if (!output) throw new Error("--out requires a path");
      continue;
    }
    throw new Error(`unknown argument ${JSON.stringify(arg)}`);
  }
  if (args.includes("--help") || args.includes("-h")) {
    console.log(`Clark TUI capability simulation

Usage:
  node harness/clark-tui-product.mjs [--out PATH]
  node harness/clark-tui-product.mjs --require-complete

The simulation is fully local and validates Clark-owned behavior requirements.
--require-complete remains red until every required Clark capability is implemented.`);
    return;
  }

  const receipt = await evaluateContract();
  if (output) {
    const pathname = path.resolve(process.cwd(), output);
    await mkdir(path.dirname(pathname), { recursive: true });
    await writeFile(pathname, `${JSON.stringify(receipt, null, 2)}\n`);
  }
  if (args.includes("--json")) {
    console.log(JSON.stringify(receipt, null, 2));
  } else {
    console.log(`Clark TUI product contract: ${receipt.summary.implemented_count}/${receipt.summary.feature_count} capability groups implemented; ${receipt.summary.gap_count} explicit gaps; ${receipt.summary.command_count} intentional commands.`);
    for (const feature of receipt.features.filter((item) => item.state === "gap")) {
      console.log(`GAP ${feature.id}: ${feature.first_failure}`);
    }
  }
  if (args.includes("--require-complete") && !receipt.summary.complete) {
    throw new Error(`Clark TUI product contract is incomplete: ${receipt.summary.gap_count} capability groups remain`);
  }
}

const invokedDirectly = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedDirectly) {
  main().catch((error) => {
    console.error(error.stack || error.message || String(error));
    process.exitCode = 1;
  });
}
