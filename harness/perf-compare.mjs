// Mechanical before/after comparison of two performance runs.
//
// Reads only `summary.json` from each run directory, so comparison never has to
// understand the per-sample streams. Refuses to compare runs that are not
// comparable — a quiet-machine baseline diffed against a loaded-machine
// candidate produces a confident number that means nothing, and that mistake is
// easy to make weeks later when the directory names have blurred together.
//
//   node harness/perf-compare.mjs <baselineDir> <candidateDir>
import { readFileSync, existsSync } from "node:fs";
import { resolve } from "node:path";

/** A candidate must beat the baseline by more than this to count as a win, and
 *  lose by more than this to count as a regression. Frame timing is noisy; a
 *  3% wobble is not a result. */
const NOISE_RATIO = 0.10;

function loadSummary(dir) {
  const path = resolve(dir, "summary.json");
  if (!existsSync(path)) {
    throw new Error(`no summary.json in ${dir} — was the run completed?`);
  }
  return JSON.parse(readFileSync(path, "utf8"));
}

/** Fields that must agree, or the two runs are measuring different things. */
function comparabilityKey(summary) {
  return {
    scenario: summary.scenario ?? null,
    // A 120 Hz run and a 60 Hz run have different budgets and different noise.
    baselinePeriodMs: summary.baselinePeriodMs === undefined
      ? null
      : Math.round(summary.baselinePeriodMs),
    // Instrumentation that perturbs (inspector open, host tracing spans) makes
    // a run useful for attribution and useless as a baseline.
    perturbed: summary.perturbed ?? "none",
    // A substituted metric is not the same metric.
    substituted: (summary.capabilities?.missingEntryTypes ?? []).join(","),
  };
}

function assertComparable(baseline, candidate) {
  const a = comparabilityKey(baseline);
  const b = comparabilityKey(candidate);
  const mismatched = Object.keys(a).filter((key) => String(a[key]) !== String(b[key]));
  if (mismatched.length > 0) {
    const detail = mismatched
      .map((key) => `  ${key}: baseline=${a[key]!== null ? a[key] : "null"} candidate=${b[key] !== null ? b[key] : "null"}`)
      .join("\n");
    throw new Error(`these runs are not comparable:\n${detail}`);
  }
}

function formatDelta(before, after) {
  if (before === 0) return after === 0 ? "0" : "new";
  const ratio = (after - before) / Math.abs(before);
  const sign = ratio > 0 ? "+" : "";
  return `${sign}${(ratio * 100).toFixed(1)}%`;
}

function classify(metric, before, after) {
  if (before === 0 && after === 0) return "same";
  const ratio = before === 0 ? 1 : (after - before) / Math.abs(before);
  if (Math.abs(ratio) <= NOISE_RATIO) return "same";
  // Every metric here is a cost: lower is better.
  return ratio < 0 ? "better" : "WORSE";
}

function compare(baselineDir, candidateDir) {
  const baseline = loadSummary(baselineDir);
  const candidate = loadSummary(candidateDir);
  assertComparable(baseline, candidate);

  const names = [...new Set([
    ...Object.keys(baseline.metrics ?? {}),
    ...Object.keys(candidate.metrics ?? {}),
  ])].sort();

  const rows = [];
  const regressions = [];
  const budgetFlips = [];

  for (const name of names) {
    const before = baseline.metrics?.[name];
    const after = candidate.metrics?.[name];
    if (!before || !after) {
      rows.push({ name, note: before ? "removed" : "added" });
      continue;
    }
    // A metric with no samples is "not measured in this scenario", not zero.
    // Printing 0.000 for the host-emit timings in a scenario with no host reads
    // as "emit latency is zero", which is the opposite of the truth.
    if (before.n === 0 && after.n === 0) {
      rows.push({ name, note: "no samples" });
      continue;
    }
    if (before.n === 0 || after.n === 0) {
      rows.push({ name, note: `sample count changed (${before.n} -> ${after.n})` });
      continue;
    }
    const verdict = classify(name, before.p95, after.p95);
    rows.push({
      name,
      unit: after.unit,
      beforeP95: before.p95,
      afterP95: after.p95,
      delta: formatDelta(before.p95, after.p95),
      verdict,
    });
    if (verdict === "WORSE") regressions.push(name);
    if (before.pass === true && after.pass === false) budgetFlips.push(name);
  }

  // Growth slopes are the "does it degrade over a session" signal; a fix that
  // improves p95 but leaves the slope intact has not fixed the real problem.
  const growth = [...new Set([
    ...Object.keys(baseline.growth ?? {}),
    ...Object.keys(candidate.growth ?? {}),
  ])].sort().map((name) => ({
    name,
    before: baseline.growth?.[name] ?? null,
    after: candidate.growth?.[name] ?? null,
  }));

  return { baseline, candidate, rows, growth, regressions, budgetFlips };
}

function render(result) {
  const pad = (value, width) => String(value).padStart(width);
  console.log(`scenario: ${result.candidate.scenario}`);
  console.log(`baseline period: ${result.candidate.baselinePeriodMs?.toFixed?.(2) ?? "?"} ms\n`);
  console.log(`${"metric".padEnd(34)}${pad("before", 12)}${pad("after", 12)}${pad("delta", 10)}  verdict`);
  for (const row of result.rows) {
    if (row.note) {
      console.log(`${row.name.padEnd(34)}${pad("-", 12)}${pad("-", 12)}${pad("-", 10)}  ${row.note}`);
      continue;
    }
    console.log(
      row.name.padEnd(34)
      + pad(row.beforeP95.toFixed(3), 12)
      + pad(row.afterP95.toFixed(3), 12)
      + pad(row.delta, 10)
      + `  ${row.verdict}`,
    );
  }
  if (result.growth.length > 0) {
    console.log("\ngrowth per timeline item (a positive slope means it degrades over a session)");
    for (const row of result.growth) {
      console.log(
        `  ${row.name.padEnd(40)}${pad(row.before?.toFixed?.(4) ?? "-", 12)}`
        + pad(row.after?.toFixed?.(4) ?? "-", 12),
      );
    }
  }
  if (result.budgetFlips.length > 0) {
    console.log(`\nBUDGET REGRESSION: ${result.budgetFlips.join(", ")}`);
  }
  if (result.regressions.length > 0) {
    console.log(`\nREGRESSED (>${NOISE_RATIO * 100}%): ${result.regressions.join(", ")}`);
  }
  const failed = result.regressions.length > 0 || result.budgetFlips.length > 0;
  console.log(failed ? "\nFAIL" : "\nOK");
  return failed ? 1 : 0;
}

const [baselineDir, candidateDir] = process.argv.slice(2);
if (!baselineDir || !candidateDir) {
  console.error("usage: node harness/perf-compare.mjs <baselineDir> <candidateDir>");
  process.exit(2);
}
process.exit(render(compare(baselineDir, candidateDir)));
