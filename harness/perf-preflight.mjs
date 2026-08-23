// Machine-quiescence gate for performance measurement.
//
// A frame-timing number taken on a busy machine is not a measurement of the
// app; it is a measurement of the machine. This refuses to let a run start
// unless the host is quiet, and reports the same readings again afterwards so a
// run that went bad mid-flight can be thrown out rather than believed.
//
// Run standalone to see where the machine stands:
//   node harness/perf-preflight.mjs
// Or import { preflight, assertQuiet } from a driver.
import { execFileSync } from "node:child_process";
import os from "node:os";

/** Thresholds. Deliberately strict: the cost of a rejected run is minutes, the
 *  cost of a believed-but-invalid baseline is every decision made from it. */
export const LIMITS = {
  loadAverage1m: 4.0,
  swapUsedGb: 2.0,
  windowServerCpuPercent: 10,
  minMemoryFreePercent: 20,
};

function sh(command, args) {
  try {
    return execFileSync(command, args, { encoding: "utf8", timeout: 20_000 });
  } catch {
    return "";
  }
}

function swapUsedGb() {
  // vm.swapusage: total = 22528.00M  used = 21291.81M  free = 1236.19M
  const match = /used\s*=\s*([\d.]+)([MG])/.exec(sh("sysctl", ["vm.swapusage"]));
  if (!match) return null;
  const value = Number.parseFloat(match[1]);
  return match[2] === "G" ? value : value / 1024;
}

function windowServerCpuPercent() {
  // `ps` sums CPU across the process's lifetime, so sample the live rate.
  const output = sh("top", ["-l", "3", "-stats", "command,cpu", "-n", "40"]);
  const samples = output
    .split("\n")
    .filter((line) => /^WindowServer\s/.test(line.trim()))
    .map((line) => Number.parseFloat(line.trim().split(/\s+/)[1]))
    .filter((value) => Number.isFinite(value));
  if (samples.length === 0) return null;
  // Drop the first sample: top's initial reading is cumulative, not a rate.
  const live = samples.slice(1);
  return live.length === 0 ? samples[0] : Math.max(...live);
}

/** Compilers and bundlers actually executing.
 *
 *  Matched on the executable's basename, not the full command line: a shell
 *  whose arguments merely mention "cargo" is not a running compiler, and
 *  matching command lines makes this fire on the harness that invoked it. */
// Only names that actually appear as an executable basename. vite, tsc,
// vitest, and rollup run under `node`, so listing them here would be dead
// entries giving false confidence — and matching bare `node` would flag this
// harness itself. Node-hosted bundlers are effectively invisible to this
// check; the heavy native compilers are the ones that matter.
const BUSY_EXECUTABLES = new Set([
  "cargo", "rustc", "xcodebuild", "clang", "ld", "swift-frontend",
  "esbuild", "cargo-nextest",
]);

function busyBuilders() {
  const output = sh("ps", ["-A", "-o", "comm="]);
  const counts = new Map();
  for (const line of output.split("\n")) {
    const name = line.trim().split("/").pop();
    if (!name || !BUSY_EXECUTABLES.has(name)) continue;
    counts.set(name, (counts.get(name) ?? 0) + 1);
  }
  return [...counts.entries()].map(([name, count]) => `${name} x${count}`);
}

/** Share of memory the kernel reports as available.
 *
 *  `os.freemem()` is the wrong signal on macOS: the kernel keeps most of RAM
 *  as cache, so a healthy machine reports single-digit "free" memory.
 *  `memory_pressure` reports the number that actually reflects pressure. */
function memoryFreePercent() {
  const match = /System-wide memory free percentage:\s*(\d+)/
    .exec(sh("memory_pressure", []));
  return match ? Number.parseInt(match[1], 10) : null;
}

function spotlightBusy() {
  return /Indexing enabled/i.test(sh("mdutil", ["-s", "/"]))
    && sh("pgrep", ["-fl", "mdworker|mds_stores"]).trim().length > 0;
}

function timeMachineBusy() {
  return /Running\s*=\s*1/.test(sh("tmutil", ["status"]));
}

function onBatteryOrThrottled() {
  const power = sh("pmset", ["-g", "ps"]);
  const thermal = sh("pmset", ["-g", "therm"]);
  const onBattery = /Battery Power/.test(power);
  const throttled = /CPU_Speed_Limit\s*=\s*(?!100)/.test(thermal);
  return { onBattery, throttled };
}

export function preflight() {
  const [load1m] = os.loadavg();
  const swap = swapUsedGb();
  const windowServer = windowServerCpuPercent();
  const builders = busyBuilders();
  const { onBattery, throttled } = onBatteryOrThrottled();
  const memoryFree = memoryFreePercent();

  const failures = [];
  if (load1m > LIMITS.loadAverage1m) {
    failures.push(`1-minute load average ${load1m.toFixed(1)} exceeds ${LIMITS.loadAverage1m}`);
  }
  if (swap !== null && swap > LIMITS.swapUsedGb) {
    failures.push(`${swap.toFixed(1)} GB of swap in use, limit ${LIMITS.swapUsedGb} GB`);
  }
  if (windowServer !== null && windowServer > LIMITS.windowServerCpuPercent) {
    failures.push(
      `WindowServer at ${windowServer.toFixed(0)}% CPU, limit ${LIMITS.windowServerCpuPercent}%`,
    );
  }
  if (builders.length > 0) {
    failures.push(`compilers or bundlers running: ${builders.join(", ")}`);
  }
  if (spotlightBusy()) failures.push("Spotlight is indexing");
  if (timeMachineBusy()) failures.push("Time Machine is running");
  if (onBattery) failures.push("on battery power");
  if (throttled) failures.push("CPU is thermally throttled");
  if (memoryFree !== null && memoryFree < LIMITS.minMemoryFreePercent) {
    failures.push(`only ${memoryFree}% of memory free, limit ${LIMITS.minMemoryFreePercent}%`);
  }

  return {
    at: new Date().toISOString(),
    readings: {
      loadAverage: os.loadavg(),
      swapUsedGb: swap,
      windowServerCpuPercent: windowServer,
      memoryFreePercent: memoryFree,
      busyProcesses: builders,
      onBattery,
      throttled,
      cpus: os.cpus().length,
    },
    failures,
    quiet: failures.length === 0,
  };
}

/** Throw unless the machine is fit to measure on. */
export function assertQuiet(label = "run") {
  const result = preflight();
  if (!result.quiet) {
    const detail = result.failures.map((line) => `  - ${line}`).join("\n");
    throw new Error(
      `refusing to start ${label}: the machine is not quiet enough to measure.\n${detail}\n`
      + "Numbers taken now would describe the machine, not the app.",
    );
  }
  return result;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const result = preflight();
  console.log(JSON.stringify(result, null, 2));
  console.log(result.quiet
    ? "\nQUIET — safe to measure."
    : `\nNOT QUIET — ${result.failures.length} blocker(s) above.`);
  process.exit(result.quiet ? 0 : 1);
}
