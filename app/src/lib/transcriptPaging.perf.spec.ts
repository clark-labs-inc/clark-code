import { describe, expect, it } from "vitest";
import type { Snapshot, TimelineItem } from "../core-bridge/types";
import {
  preparePagedSnapshot,
  TRANSCRIPT_PAGE_HARD_BYTES,
  TRANSCRIPT_TAIL_ITEMS,
  transcriptPageBatches,
} from "./transcriptPaging";
import { utf8ByteLength } from "./snapshotUpload";

declare const process: {
  env: Record<string, string | undefined>;
  memoryUsage(): { rss: number };
};

const enabled = process.env.CLARK_LARGE_TRANSCRIPT_PERF === "1";

function logicalTranscript(payloadMiB: number): Snapshot {
  const text = "x".repeat(1024 * 1024);
  const message: TimelineItem = {
    item: "message",
    run: "run",
    role: "user",
    blocks: [{ type: "text", text }],
  };
  const count = payloadMiB + TRANSCRIPT_TAIL_ITEMS;
  return {
    runs: { run: { id: "run", status: "done" } },
    timeline: Array.from({ length: count }, () => message),
    model_context_checkpoint: {
      transcript: { items: [], truncated: true },
      timeline_index: count,
    },
    tool_calls: {},
    artifacts: [],
    provider_incidents: {},
  };
}

describe.skipIf(!enabled)("large transcript performance gate", () => {
  for (const payloadMiB of [100, 1024]) {
    it(`${payloadMiB} MiB stays within bounded page and memory envelopes`, () => {
      const source = logicalTranscript(payloadMiB);
      const baselineRss = process.memoryUsage().rss;
      const started = performance.now();
      const plan = preparePagedSnapshot(source, 0);
      let maxBatchBytes = 0;
      let maxRss = baselineRss;
      let pageCount = 0;
      let archivedItems = 0;
      for (const batch of transcriptPageBatches(
        source,
        plan.pageStartLocal,
        plan.pageEndLocal,
      )) {
        const batchBytes = batch.reduce((total, page) => {
          pageCount += 1;
          archivedItems += page.items.length;
          const bytes = utf8ByteLength(JSON.stringify(page));
          expect(bytes).toBeLessThanOrEqual(TRANSCRIPT_PAGE_HARD_BYTES);
          return total + bytes;
        }, 0);
        maxBatchBytes = Math.max(maxBatchBytes, batchBytes);
        maxRss = Math.max(maxRss, process.memoryUsage().rss);
      }
      const elapsedMs = performance.now() - started;
      console.info(JSON.stringify({
        gate: "transcript-pages-v1",
        payloadMiB,
        archivedItems,
        pageCount,
        uploadRequests: Math.ceil(pageCount / 4),
        maxBatchBytes,
        peakRssDeltaBytes: maxRss - baselineRss,
        elapsedMs: Math.round(elapsedMs),
      }));

      expect(archivedItems).toBe(payloadMiB);
      expect(plan.head.timeline).toHaveLength(TRANSCRIPT_TAIL_ITEMS);
      expect(maxBatchBytes).toBeLessThanOrEqual(4 * TRANSCRIPT_PAGE_HARD_BYTES);
      // The logical payload grows 10x, but page construction keeps only one
      // four-page upload batch resident beyond the already-open transcript.
      expect(maxRss - baselineRss).toBeLessThan(256 * 1024 * 1024);
      expect(elapsedMs).toBeLessThan(payloadMiB === 100 ? 15_000 : 90_000);
      // Each request maps to one batched INSERT regardless of its 1-4 pages.
      expect(Math.ceil(pageCount / 4)).toBeLessThanOrEqual(Math.ceil(payloadMiB / 16));
    }, 120_000);
  }
});
