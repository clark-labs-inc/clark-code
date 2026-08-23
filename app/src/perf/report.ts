/** Turns raw samples into the one flat file the comparator reads.
 *
 *  `summary.json` is deliberately the only artifact `harness/perf-compare.mjs`
 *  consumes: a flat `metric -> {p50,p95,p99,max,n,unit,budget,pass}` map. The
 *  per-sample JSONL streams stay alongside it for when a number needs
 *  explaining, but comparison never has to understand their shape. */

import type { BlockSample } from "./blockProbe";
import type { Capabilities } from "./capabilities";
import type { FrameRun } from "./frameSampler";
import type { SnapshotPathSample, ClockOffset } from "./snapshotPath";

export interface Stats {
  n: number;
  p50: number;
  p95: number;
  p99: number;
  max: number;
  mean: number;
}

export function stats(values: number[]): Stats {
  if (values.length === 0) return { n: 0, p50: 0, p95: 0, p99: 0, max: 0, mean: 0 };
  const sorted = [...values].sort((a, b) => a - b);
  const at = (q: number) => sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))];
  return {
    n: sorted.length,
    p50: at(0.5),
    p95: at(0.95),
    p99: at(0.99),
    max: sorted[sorted.length - 1],
    mean: sorted.reduce((sum, value) => sum + value, 0) / sorted.length,
  };
}

export interface Metric extends Stats {
  unit: string;
  budget?: number;
  /** Undefined when no budget applies; never guessed. */
  pass?: boolean;
}

function metric(
  values: number[],
  unit: string,
  budget?: number,
  gate: "p95" | "p99" = "p95",
): Metric {
  const base = stats(values);
  // Budgets default to p95: a single outlier should not fail a run, and a
  // median hides the stutter the user actually notices. Block time gates at
  // p99, because rare-but-long main-thread stalls ARE the reported symptom.
  return {
    ...base,
    unit,
    budget,
    pass: budget === undefined ? undefined : base[gate] <= budget,
  };
}

export interface FrameBudgets {
  /** Share of display periods that produced no frame. */
  droppedRatio: number;
  droppedFrames: number;
  longestGapPeriods: number;
}

/** Frames the engine never delivered, in units of the observed period. */
export function frameLoss(run: FrameRun): FrameBudgets {
  const period = run.baselinePeriodMs;
  let dropped = 0;
  let longest = 0;
  for (const sample of run.samples.slice(1)) {
    const periods = Math.floor(sample.dt / period);
    if (periods > 1) dropped += periods - 1;
    longest = Math.max(longest, periods);
  }
  const expected = run.samples.length + dropped;
  return {
    droppedRatio: expected === 0 ? 0 : dropped / expected,
    droppedFrames: dropped,
    longestGapPeriods: longest,
  };
}

/**
 * Least-squares slope of a duration against transcript length.
 *
 * This is the number that separates a constant per-frame cost from one that
 * grows with the conversation. A slope near zero means the work is bounded; a
 * positive slope means every additional turn makes the app slower, which is a
 * different class of bug and needs a different fix.
 */
export function growthSlope(points: Array<{ x: number; y: number }>): number {
  const usable = points.filter((p) => Number.isFinite(p.x) && Number.isFinite(p.y));
  if (usable.length < 3) return 0;
  const n = usable.length;
  const meanX = usable.reduce((sum, p) => sum + p.x, 0) / n;
  const meanY = usable.reduce((sum, p) => sum + p.y, 0) / n;
  let covariance = 0;
  let variance = 0;
  for (const point of usable) {
    covariance += (point.x - meanX) * (point.y - meanY);
    variance += (point.x - meanX) ** 2;
  }
  return variance === 0 ? 0 : covariance / variance;
}

export interface Budgets {
  droppedRatio: number;
  blockP99Ms: number;
  blockMaxMs: number;
  emitToArriveP95Ms: number;
  arriveToCommitP95Ms: number;
  snapshotBytes: number;
}

/** Defaults chosen for a 60 Hz baseline. A 120 Hz run should halve the
 *  frame-derived ones; the caller owns that, not this module. */
export const DEFAULT_BUDGETS: Budgets = {
  droppedRatio: 0.05,
  blockP99Ms: 50,
  blockMaxMs: 100,
  emitToArriveP95Ms: 33,
  arriveToCommitP95Ms: 8,
  snapshotBytes: 512 * 1024,
};

export interface Summary {
  scenario: string;
  capabilities: Capabilities;
  clock: ClockOffset;
  baselinePeriodMs: number;
  frameLoss: FrameBudgets;
  metrics: Record<string, Metric>;
  /** Growth of per-frame cost against transcript length. */
  growth: Record<string, number>;
  passed: boolean;
}

export function buildSummary(input: {
  scenario: string;
  capabilities: Capabilities;
  frames: FrameRun;
  blocks: BlockSample[];
  snapshotPath: SnapshotPathSample[];
  clock: ClockOffset;
  budgets?: Partial<Budgets>;
}): Summary {
  const budgets = { ...DEFAULT_BUDGETS, ...input.budgets };
  const loss = frameLoss(input.frames);
  const path = input.snapshotPath;
  const finite = (values: Array<number | null>) =>
    values.filter((value): value is number => value !== null && Number.isFinite(value));

  const metrics: Record<string, Metric> = {
    frameIntervalMs: metric(input.frames.samples.slice(1).map((s) => s.dt), "ms"),
    blockMs: metric(input.blocks.map((b) => b.ms), "ms", budgets.blockP99Ms, "p99"),
    emitToArriveMs: metric(finite(path.map((s) => s.emitToArriveMs)), "ms", budgets.emitToArriveP95Ms),
    arriveToCommitMs: metric(
      finite(path.map((s) => s.arriveToCommitMs)),
      "ms",
      budgets.arriveToCommitP95Ms,
    ),
    commitToPaintMs: metric(finite(path.map((s) => s.commitToPaintMs)), "ms"),
    snapshotBytes: metric(finite(path.map((s) => s.bytes)), "bytes", budgets.snapshotBytes),
    timelineLen: metric(path.map((s) => s.timelineLen), "items"),
  };

  metrics.droppedFrameRatio = {
    ...stats([loss.droppedRatio]),
    unit: "ratio",
    budget: budgets.droppedRatio,
    pass: loss.droppedRatio <= budgets.droppedRatio,
  };

  const growth = {
    arriveToCommitMsPerTimelineItem: growthSlope(
      path
        .filter((s) => s.arriveToCommitMs !== null)
        .map((s) => ({ x: s.timelineLen, y: s.arriveToCommitMs as number })),
    ),
    snapshotBytesPerTimelineItem: growthSlope(
      path
        .filter((s) => s.bytes !== null)
        .map((s) => ({ x: s.timelineLen, y: s.bytes as number })),
    ),
  };

  return {
    scenario: input.scenario,
    capabilities: input.capabilities,
    clock: input.clock,
    baselinePeriodMs: input.frames.baselinePeriodMs,
    frameLoss: loss,
    metrics,
    growth,
    passed: Object.values(metrics).every((m) => m.pass !== false),
  };
}
