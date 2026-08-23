import { describe, expect, it } from "vitest";
import { emptySnapshot, normalizeSnapshot, type Snapshot } from "./types";
import {
  RESERVED_MARKERS_FOR_TEST,
  RESERVED_MARKER_ANCHOR_SOURCE,
} from "./providerOutputQuarantine";

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

describe("marker pre-filter", () => {
  it("anchors are underscore-free runs of the markers they guard", () => {
    // The pre-filter's soundness rests on this: normalization only removes
    // underscore characters and folds case, so an underscore-free run of a
    // marker survives into the original string. If a marker is ever added
    // without an anchor, the pre-filter would start skipping real detections —
    // this test is what stops that.
    const anchors = RESERVED_MARKER_ANCHOR_SOURCE
      .split("|")
      .filter((part) => !part.startsWith("["));
    expect(anchors.length).toBeGreaterThan(0);
    for (const anchor of anchors) {
      expect(anchor).not.toContain("_");
      const guarded = RESERVED_MARKERS_FOR_TEST.filter((marker) => marker.includes(anchor));
      expect(guarded.length, `no marker contains anchor ${anchor}`).toBeGreaterThan(0);
    }
    for (const marker of RESERVED_MARKERS_FOR_TEST) {
      const covered = anchors.some((anchor) => marker.includes(anchor));
      expect(covered, `marker ${marker} has no anchor, so the pre-filter would skip it`).toBe(true);
    }
  });

  it("still detects markers the pre-filter must not skip", () => {
    const cases = [
      "begin_of_sentence",
      "BEGIN_OF_SENTENCE",
      "begin__of__sentence",
      "begin▁of▁sentence",
      "prefix require_escalated_model suffix",
      "ExPiRaTiOn_PlAcEhOlDeR",
      "skillconstraint_hard",
    ];
    for (const text of cases) {
      const snapshot: Snapshot = {
        ...emptySnapshot(),
        timeline: [{ item: "message", run: "r", role: "agent", blocks: [{ type: "text", text }] }],
      } as Snapshot;
      expect(normalizeSnapshot(snapshot).timeline, `should reject: ${text}`).toEqual([]);
    }
  });

  it("keeps clean text that merely resembles a marker", () => {
    const cases = [
      "the sentence ends here",
      "placeholder text for the form",
      "escalated to the on-call engineer",
      "a skillconstraint on its own is not the reserved form",
    ];
    for (const text of cases) {
      const snapshot: Snapshot = {
        ...emptySnapshot(),
        timeline: [{ item: "message", run: "r", role: "agent", blocks: [{ type: "text", text }] }],
      } as Snapshot;
      expect(normalizeSnapshot(snapshot).timeline.length, `should keep: ${text}`).toBe(1);
    }
  });
});
