import { describe, expect, it } from "vitest";
import type { Snapshot } from "../core-bridge/types";
import { historyFingerprintHead } from "./historyFingerprint";

describe("historyFingerprintHead", () => {
  it("removes an already immutable prefix while a new reply is streaming", () => {
    const snapshot: Snapshot = { runs: { run: { id: "run", status: "running" } },
      timeline: Array.from({ length: 400 }, (_, index) => ({ item: "message", run: "run", role: "agent",
        blocks: [{ type: "text", text: `turn-${index}` }] })),
      tool_calls: {}, artifacts: [], provider_incidents: {},
    };
    const head = historyFingerprintHead(snapshot, 240);
    expect(head.timeline_offset).toBe(240);
    expect(head.timeline).toHaveLength(160);
    expect(snapshot.timeline).toHaveLength(400);
  });
  it("hashes a compacted history through the retained head", () => {
    const snapshot: Snapshot = {
      runs: {},
      timeline: Array.from({ length: 400 }, (_, index) => ({
        item: "message" as const,
        run: "run",
        role: "user" as const,
        blocks: [{ type: "text" as const, text: `turn-${index}` }],
      })),
      model_context_checkpoint: { timeline_index: 400, transcript: { items: [], truncated: true } },
      tool_calls: {}, artifacts: [], provider_incidents: {},
    };

    const head = historyFingerprintHead(snapshot, 0);
    expect(head.timeline_offset).toBe(240);
    expect(head.timeline).toHaveLength(160);
    expect(head.timeline[0]).toEqual(snapshot.timeline[240]);
    expect(snapshot.timeline).toHaveLength(400);
  });

  it("keeps active snapshots whole while their earlier rows can still change", () => {
    const snapshot: Snapshot = {
      runs: { run: { id: "run", status: "running" } },
      timeline: [{ item: "message", run: "run", role: "agent", blocks: [{ type: "text", text: "live" }] }],
      model_context_checkpoint: { timeline_index: 1, transcript: { items: [], truncated: true } },
      tool_calls: {}, artifacts: [], provider_incidents: {},
    };
    expect(historyFingerprintHead(snapshot, 0)).toBe(snapshot);
  });
});
