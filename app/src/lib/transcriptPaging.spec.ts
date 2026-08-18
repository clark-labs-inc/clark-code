import { describe, expect, it } from "vitest";
import type { Snapshot } from "../core-bridge/types";
import {
  preparePagedSnapshot,
  TRANSCRIPT_PAGE_HARD_BYTES,
  TRANSCRIPT_TAIL_ITEMS,
  transcriptPageBatches,
} from "./transcriptPaging";
import { utf8ByteLength } from "./snapshotUpload";

function snapshot(messageCount: number, text = "hello"): Snapshot {
  return {
    runs: { run: { id: "run", status: "done" } },
    timeline: Array.from({ length: messageCount }, (_, index) => ({
      item: "message" as const,
      run: "run",
      role: "user" as const,
      blocks: [{ type: "text" as const, text: `${text}-${index}` }],
    })),
    model_context_checkpoint: {
      transcript: { items: [], truncated: true },
      timeline_index: messageCount,
    },
    tool_calls: {},
    artifacts: [],
    provider_incidents: {},
  };
}

describe("incremental transcript paging", () => {
  it("keeps a fixed live tail and emits bounded batches", () => {
    const source = snapshot(400);
    const plan = preparePagedSnapshot(source, 0);
    const batches = [...transcriptPageBatches(
      source,
      plan.pageStartLocal,
      plan.pageEndLocal,
    )];

    expect(plan.head.timeline_offset).toBe(240);
    expect(plan.head.timeline).toHaveLength(TRANSCRIPT_TAIL_ITEMS);
    expect(batches.every((batch) => batch.length <= 4)).toBe(true);
    expect(batches.flat().reduce((count, page) => count + page.items.length, 0)).toBe(240);
    expect(batches.flat().every((page) => (
      utf8ByteLength(JSON.stringify(page)) <= TRANSCRIPT_PAGE_HARD_BYTES
    ))).toBe(true);
  });

  it("only stages the delta after the cloud's sealed prefix", () => {
    const source = snapshot(480);
    const plan = preparePagedSnapshot(source, 240);
    const pages = [...transcriptPageBatches(
      source,
      plan.pageStartLocal,
      plan.pageEndLocal,
    )].flat();

    expect(pages[0].startIndex).toBe(240);
    expect(pages.reduce((count, page) => count + page.items.length, 0)).toBe(80);
    expect(plan.sealedThrough).toBe(320);
    expect(plan.head.timeline).toHaveLength(160);
  });

  it("splits pages by UTF-8 wire size before the service boundary", () => {
    const source = snapshot(170, "x".repeat(1024 * 1024));
    const plan = preparePagedSnapshot(source, 0);
    const pages = [...transcriptPageBatches(
      source,
      plan.pageStartLocal,
      plan.pageEndLocal,
    )].flat();

    expect(pages).toHaveLength(2);
    expect(pages.map((page) => page.items.length)).toEqual([5, 5]);
  });

  it("seals a large singleton record that exceeds the ordinary page target", () => {
    const source = snapshot(161);
    source.timeline[0] = {
      item: "message",
      run: "run",
      role: "user",
      blocks: [{ type: "text", text: "x".repeat(12 * 1024 * 1024) }],
    };
    const plan = preparePagedSnapshot(source, 0);
    const batches = [...transcriptPageBatches(
      source,
      plan.pageStartLocal,
      plan.pageEndLocal,
    )];
    const wireBytes = utf8ByteLength(JSON.stringify(batches[0][0]));

    expect(batches).toHaveLength(1);
    expect(batches[0]).toHaveLength(1);
    expect(wireBytes).toBeGreaterThan(8 * 1024 * 1024);
    expect(wireBytes).toBeLessThanOrEqual(TRANSCRIPT_PAGE_HARD_BYTES);
  });
});
