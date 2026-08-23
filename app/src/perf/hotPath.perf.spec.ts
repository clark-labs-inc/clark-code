/** Microbenchmarks for the pure work on the per-snapshot path.
 *
 *  Run: CLARK_HOT_PATH_BENCH=1 npx vitest run src/perf/hotPath.perf.spec.ts
 *
 *  These functions all run on the arrival of every host snapshot — up to ~62
 *  times a second per live session. Two things matter about each: the absolute
 *  cost at a realistic transcript size, and how that cost SCALES. A function
 *  whose per-call cost grows with transcript length turns a conversation into a
 *  quadratic workload, which is the difference between "slow" and "gets slower
 *  the longer you use it".
 *
 *  Gated off by default: it is a measurement, not an assertion, and its numbers
 *  are meaningless on a loaded machine.
 */
import { describe, expect, it } from "vitest";

import { quarantineSnapshotProviderOutput } from "../core-bridge/providerOutputQuarantine";
import { emptySnapshot } from "../core-bridge/types";
import type { Snapshot, TimelineItem, ToolCall } from "../core-bridge/types";
import { mergeHistory } from "../store/sessionStore";
import { summarizeEdits } from "../lib/diff";
import { streamingReplyPlaceholderCount } from "../surfaces/StreamingReply";

// Vitest runs in Node; the app tsconfig is browser-scoped, so reach the flag
// without pulling in @types/node.
const enabled = (globalThis as { process?: { env?: Record<string, string | undefined> } })
  .process?.env?.CLARK_HOT_PATH_BENCH === "1";

/** A realistic unified diff, the shape an edit tool call actually carries. */
function diff(path: string, lines: number): string {
  const body: string[] = [
    `diff --git a/${path} b/${path}`,
    "index 1111111..2222222 100644",
    `--- a/${path}`,
    `+++ b/${path}`,
    `@@ -1,${lines} +1,${lines} @@`,
  ];
  for (let i = 0; i < lines; i += 1) {
    body.push(i % 3 === 0 ? `+  const added${i} = ${i};` : `-  const removed${i} = ${i};`);
  }
  return body.join("\n");
}

const PROSE = "Walking through the change in some detail, with enough prose to "
  + "resemble a real assistant turn rather than a placeholder. ".repeat(8);

/** A snapshot with `turns` completed turns, half of them carrying edit diffs. */
function snapshotOf(turns: number, diffLines = 200): Snapshot {
  const timeline: TimelineItem[] = [];
  const toolCalls: Record<string, ToolCall> = {};
  for (let i = 0; i < turns; i += 1) {
    timeline.push({
      item: "message",
      run: `run-${i}`,
      role: "user",
      blocks: [{ type: "text", text: `turn ${i}: keep going` }],
    } as TimelineItem);
    timeline.push({
      item: "message",
      run: `run-${i}`,
      role: "agent",
      phase: "final_answer",
      blocks: [{ type: "text", text: PROSE }],
    } as TimelineItem);
    if (i % 2 === 0) {
      const id = `call-${i}`;
      toolCalls[id] = {
        id,
        kind: "edit",
        title: `edit file-${i}.ts`,
        status: "done",
        content: [{ type: "text", text: diff(`src/file-${i}.ts`, diffLines) }],
        locations: [{ path: `src/file-${i}.ts` }],
        raw_input: { path: `src/file-${i}.ts`, old_string: "a".repeat(400), new_string: "b".repeat(400) },
      } as unknown as ToolCall;
      timeline.push({ item: "tool_call", run: `run-${i}`, id } as TimelineItem);
    }
  }
  return { ...emptySnapshot(), session: "bench", timeline, tool_calls: toolCalls };
}

function bytesOf(snapshot: Snapshot): number {
  return JSON.stringify(snapshot).length;
}

/** Median of `runs` timed calls, in milliseconds. */
function timed(label: string, runs: number, work: () => unknown): number {
  // One warm-up so JIT tiering is not part of the first measurement.
  work();
  const samples: number[] = [];
  for (let i = 0; i < runs; i += 1) {
    const started = performance.now();
    work();
    samples.push(performance.now() - started);
  }
  samples.sort((a, b) => a - b);
  const median = samples[Math.floor(samples.length / 2)];
  void label;
  return median;
}

const SIZES = [10, 40, 160, 320];

describe.skipIf(!enabled)("per-snapshot hot path", () => {
  it("attributes the quarantine scan across its four inputs", () => {
    // Where the scan spends its time decides what a cache has to key on. The
    // marker test allocates three copies of every string it inspects
    // (NFKC normalize, lowercase, regex replace), so the answer tracks total
    // inspected text rather than node count.
    const marker = (value: string) => {
      const normalized = value.normalize("NFKC").toLowerCase().replace(/[_▁]+/gu, "_");
      return normalized.includes("begin_of_sentence");
    };
    const rows: Array<Record<string, string | number>> = [];
    for (const turns of SIZES) {
      const snapshot = snapshotOf(turns);
      const agentBlocks = snapshot.timeline.filter(
        (item) => item.item === "message" && item.role === "agent",
      );
      const calls = Object.values(snapshot.tool_calls);
      rows.push({
        turns,
        agentProseMs: Number(timed("prose", 20, () => {
          for (const item of agentBlocks) {
            for (const block of (item as { blocks: Array<{ text?: string }> }).blocks) {
              if (block.text) marker(block.text);
            }
          }
        }).toFixed(3)),
        toolContentMs: Number(timed("toolContent", 20, () => {
          for (const call of calls) {
            for (const block of call.content as Array<{ text?: string }>) {
              if (block.text) marker(block.text);
            }
          }
        }).toFixed(3)),
        rawInputMs: Number(timed("rawInput", 20, () => {
          for (const call of calls) {
            for (const value of Object.values(call.raw_input ?? {})) {
              if (typeof value === "string") marker(value);
            }
          }
        }).toFixed(3)),
      });
    }
    // eslint-disable-next-line no-console
    console.log("\nquarantine scan, split by input (median ms per call)");
    // eslint-disable-next-line no-console
    console.table(rows);
    expect(rows.length).toBe(SIZES.length);
  });

  it("isolates the quarantine cost by removing one input at a time", () => {
    // The earlier split measured a hand-rolled approximation of the scan and
    // accounted for only half the real cost. Ablation measures the real
    // function instead: strip one input, re-run, and the drop is that input's
    // true share — including whatever machinery walks it.
    const rows: Array<Record<string, string | number>> = [];
    for (const turns of SIZES) {
      const full = snapshotOf(turns);
      const noRawInput = {
        ...full,
        tool_calls: Object.fromEntries(
          Object.entries(full.tool_calls).map(([id, call]) => [id, { ...call, raw_input: undefined }]),
        ),
      } as Snapshot;
      const noToolContent = {
        ...full,
        tool_calls: Object.fromEntries(
          Object.entries(full.tool_calls).map(([id, call]) => [id, { ...call, content: [] }]),
        ),
      } as Snapshot;
      const noTimeline = { ...full, timeline: [] } as Snapshot;
      const measure = (snap: Snapshot) =>
        timed("q", 20, () => quarantineSnapshotProviderOutput(snap));
      const base = measure(full);
      rows.push({
        turns,
        fullMs: Number(base.toFixed(3)),
        withoutRawInputMs: Number(measure(noRawInput).toFixed(3)),
        withoutToolContentMs: Number(measure(noToolContent).toFixed(3)),
        withoutTimelineMs: Number(measure(noTimeline).toFixed(3)),
      });
    }
    // eslint-disable-next-line no-console
    console.log("\nquarantine ablation (median ms per call; a big drop names the culprit)");
    // eslint-disable-next-line no-console
    console.table(rows);
    expect(rows.length).toBe(SIZES.length);
  });

  it("reports cost and scaling for each function on the arrival path", () => {
    const rows: Array<Record<string, string | number>> = [];
    for (const turns of SIZES) {
      const snapshot = snapshotOf(turns);
      const prefix = snapshotOf(Math.floor(turns / 2));
      const live = snapshotOf(Math.ceil(turns / 2));
      const editCalls = Object.values(snapshot.tool_calls);
      const streamingText = PROSE.repeat(4);

      rows.push({
        turns,
        items: snapshot.timeline.length,
        kb: Math.round(bytesOf(snapshot) / 1024),
        quarantineMs: Number(timed("quarantine", 20, () =>
          quarantineSnapshotProviderOutput(snapshot)).toFixed(3)),
        summarizeEditsMs: Number(timed("summarizeEdits", 20, () =>
          summarizeEdits(editCalls)).toFixed(3)),
        mergeHistoryMs: Number(timed("mergeHistory", 20, () =>
          mergeHistory(prefix, live)).toFixed(3)),
        placeholderMs: Number(timed("placeholders", 50, () =>
          streamingReplyPlaceholderCount(streamingText)).toFixed(4)),
        serializeMs: Number(timed("JSON.stringify", 10, () =>
          JSON.stringify(snapshot)).toFixed(3)),
      });
    }

    // eslint-disable-next-line no-console
    console.log("\nper-snapshot cost by transcript size (median ms per call)");
    // eslint-disable-next-line no-console
    console.table(rows);

    const first = rows[0];
    const last = rows[rows.length - 1];
    const growth = (key: string) =>
      (Number(last[key]) / Math.max(Number(first[key]), 0.0001)).toFixed(1);
    // eslint-disable-next-line no-console
    console.log(
      `size ${first.turns} -> ${last.turns} turns (${first.kb}KB -> ${last.kb}KB):\n`
      + `  quarantine      x${growth("quarantineMs")}\n`
      + `  summarizeEdits  x${growth("summarizeEditsMs")}\n`
      + `  mergeHistory    x${growth("mergeHistoryMs")}\n`
      + `  JSON.stringify  x${growth("serializeMs")}\n`
      + `  (a 32x size increase scaling worse than x32 is superlinear)`,
    );
    expect(rows.length).toBe(SIZES.length);
  });
});
