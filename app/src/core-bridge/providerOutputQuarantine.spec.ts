import { describe, expect, it } from "vitest";
import { emptySnapshot, normalizeSnapshot, type Snapshot } from "./types";

describe("provider output quarantine", () => {
  it("removes contaminated agent output from rendering and resumed context", () => {
    const snapshot: Snapshot = {
      ...emptySnapshot(),
      timeline: [
        {
          item: "message",
          run: "run-1",
          role: "user",
          blocks: [{ type: "text", text: "keep my prompt" }],
        },
        {
          item: "message",
          run: "run-1",
          role: "agent",
          blocks: [{ type: "text", text: "safe prefix <|begin__of__sentence|> residue" }],
        },
      ],
      model_context_checkpoint: {
        timeline_index: 2,
        transcript: {
          truncated: false,
          items: [{
            item: "message",
            role: "agent",
            blocks: [{ type: "text", text: "expiration_placeholder" }],
          }],
        },
      },
    };

    const normalized = normalizeSnapshot(snapshot);

    expect(normalized.timeline).toEqual([snapshot.timeline[0]]);
    expect(normalized.model_context_checkpoint).toBeUndefined();
  });

  it("removes the Unicode tokenizer marker observed in persisted conversations", () => {
    const snapshot: Snapshot = {
      ...emptySnapshot(),
      timeline: [{
        item: "message",
        run: "run-1",
        role: "agent",
        blocks: [{ type: "text", text: "safe prefix <｜begin▁of▁sentence｜> leaked tail" }],
      }],
    };

    expect(normalizeSnapshot(snapshot).timeline).toEqual([]);
  });

  it("removes contaminated tool arguments and their timeline reference", () => {
    const snapshot: Snapshot = {
      ...emptySnapshot(),
      timeline: [{ item: "tool_call", id: "bad-tool", run: "run-1" }],
      tool_calls: {
        "bad-tool": {
          id: "bad-tool",
          tool_name: "message",
          title: "Send message",
          kind: "other",
          status: "completed",
          locations: [],
          content: [],
          raw_input: { text: "require__escalated__model" },
        },
      },
    };

    const normalized = normalizeSnapshot(snapshot);

    expect(normalized.timeline).toEqual([]);
    expect(normalized.tool_calls).toEqual({});
  });

  it("fails closed when structured provider output exceeds the scan bound", () => {
    const snapshot: Snapshot = {
      ...emptySnapshot(),
      timeline: [{ item: "tool_call", id: "oversized-tool", run: "run-1" }],
      tool_calls: {
        "oversized-tool": {
          id: "oversized-tool",
          title: "Oversized provider call",
          kind: "other",
          status: "completed",
          locations: [],
          content: [],
          raw_input: Array.from({ length: 10_001 }, () => "safe"),
        },
      },
    };

    const normalized = normalizeSnapshot(snapshot);

    expect(normalized.timeline).toEqual([]);
    expect(normalized.tool_calls).toEqual({});
  });

  it("leaves ordinary user and agent content unchanged", () => {
    const snapshot: Snapshot = {
      ...emptySnapshot(),
      timeline: [{
        item: "message",
        run: "run-1",
        role: "agent",
        blocks: [{ type: "text", text: "The nucleusinsight.com build passed." }],
      }],
    };

    expect(normalizeSnapshot(snapshot).timeline).toEqual(snapshot.timeline);
  });
});
