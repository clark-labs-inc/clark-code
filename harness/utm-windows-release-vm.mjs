#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import process from "node:process";
import { pathToFileURL } from "node:url";

import { ensureUtmVmStarted, parseUtmList } from "./utm-real-use.mjs";

const BASE_VM = "Clark QA - Windows 11 ARM";
const CLONE_PATTERN = /^Clark QA Release [1-9][0-9]*-[1-9][0-9]*$/;

function sleep(milliseconds) {
  if (milliseconds > 0) {
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);
  }
}

function defaultRun(command, args, options = {}) {
  const completed = spawnSync(command, args, {
    encoding: "utf8",
    timeout: options.timeout_ms ?? 300_000,
    maxBuffer: 4 * 1024 * 1024,
  });
  return {
    ok: completed.status === 0,
    exit_code: completed.status,
    stdout: completed.stdout || "",
    stderr: completed.stderr || completed.error?.message || "",
  };
}

export function validateReleaseVmNames({ base, clone }) {
  if (base !== undefined && base !== BASE_VM) {
    throw new Error(`release VM base must be exactly ${BASE_VM}`);
  }
  if (clone !== undefined && !CLONE_PATTERN.test(clone)) {
    throw new Error("release VM clone name is outside the per-run Clark QA namespace");
  }
  return { base, clone };
}

function listVms(run) {
  const listed = run("utmctl", ["list"]);
  if (!listed.ok) throw new Error(listed.stderr || "utmctl list failed");
  return parseUtmList(listed.stdout);
}

function waitForStatus(name, expected, run, attempts = 90) {
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    if (attempt > 0) sleep(1_000);
    const guest = listVms(run).find((item) => item.name === name);
    if (expected === "absent" ? !guest : guest?.status === expected) return guest ?? null;
  }
  throw new Error(`${name} did not reach UTM state ${expected}`);
}

export function stopReleaseVm(name, run = defaultRun) {
  const guest = listVms(run).find((item) => item.name === name);
  if (!guest || guest.status === "stopped") return guest ?? null;
  const requested = run("utmctl", ["stop", "--hide", name, "--request"], {
    timeout_ms: 30_000,
  });
  if (requested.ok) {
    try {
      return waitForStatus(name, "stopped", run, 60);
    } catch {
      // A guest that cannot complete an orderly shutdown is force-stopped
      // before cloning or deletion.
    }
  }
  const forced = run("utmctl", ["stop", "--hide", name, "--force"], {
    timeout_ms: 30_000,
  });
  if (!forced.ok) throw new Error(forced.stderr || `could not stop ${name}`);
  return waitForStatus(name, "stopped", run, 30);
}

export function preparePristineBase(base, run = defaultRun) {
  validateReleaseVmNames({ base });
  const guest = listVms(run).find((item) => item.name === base);
  if (!guest) throw new Error(`pristine Windows release base is not registered: ${base}`);
  stopReleaseVm(base, run);
  return { status: "prepared", base };
}

export function createReleaseClone({ base, clone, run = defaultRun }) {
  validateReleaseVmNames({ base, clone });
  const guests = listVms(run);
  const baseGuest = guests.find((item) => item.name === base);
  if (!baseGuest || baseGuest.status !== "stopped") {
    throw new Error("pristine Windows release base must be stopped before cloning");
  }
  if (guests.some((item) => item.name === clone)) {
    throw new Error(`refusing to reuse stale Windows release clone ${clone}`);
  }
  const cloned = run(
    "utmctl",
    ["clone", "--hide", base, "--name", clone],
    { timeout_ms: 600_000 },
  );
  if (!cloned.ok) throw new Error(cloned.stderr || `could not clone ${base}`);
  waitForStatus(clone, "stopped", run, 60);
  const started = ensureUtmVmStarted({
    vmName: clone,
    run,
    pollAttempts: 120,
    pollDelayMs: 1_000,
  });
  return { status: "started", base, clone, uuid: started.uuid };
}

export function deleteReleaseClone(clone, run = defaultRun) {
  validateReleaseVmNames({ clone });
  const guest = listVms(run).find((item) => item.name === clone);
  if (!guest) return { status: "absent", clone };
  stopReleaseVm(clone, run);
  const deleted = run("utmctl", ["delete", "--hide", clone], {
    timeout_ms: 600_000,
  });
  if (!deleted.ok) throw new Error(deleted.stderr || `could not delete ${clone}`);
  waitForStatus(clone, "absent", run, 60);
  return { status: "deleted", clone };
}

function valueArg(args, name, required = true) {
  const inline = args.find((arg) => arg.startsWith(`${name}=`));
  if (inline) return inline.slice(name.length + 1);
  const index = args.indexOf(name);
  if (index < 0) {
    if (required) throw new Error(`${name} is required`);
    return undefined;
  }
  const value = args[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value`);
  return value;
}

function main() {
  const args = process.argv.slice(2);
  const action = valueArg(args, "--action");
  let result;
  if (action === "prepare-base") {
    result = preparePristineBase(valueArg(args, "--base"));
  } else if (action === "create-clone") {
    result = createReleaseClone({
      base: valueArg(args, "--base"),
      clone: valueArg(args, "--clone"),
    });
  } else if (action === "delete-clone") {
    result = deleteReleaseClone(valueArg(args, "--clone"));
  } else {
    throw new Error(`unknown release VM action ${JSON.stringify(action)}`);
  }
  process.stdout.write(`${JSON.stringify(result)}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`${error.stack || error.message}\n`);
    process.exitCode = 1;
  }
}
