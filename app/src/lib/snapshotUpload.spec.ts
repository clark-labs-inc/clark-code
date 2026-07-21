import { describe, expect, it } from "vitest";
import type { Snapshot, ToolCall } from "../core-bridge/types";
import {
  KEEP_RECENT_TOOL_CALLS,
  prepareSnapshotForUpload,
  TEXT_PREVIEW_CHARS,
} from "./snapshotUpload";

function toolCall(id: string, textLen: number): ToolCall {
  return {
    id,
    title: `Read ${id}`,
    kind: "read",
    status: "completed",
    locations: [{ path: id }],
    content: [{ type: "text", text: "x".repeat(textLen) }],
    raw_input: { path: id },
  };
}

/** Snapshot with `count` tool calls, each carrying `textLen` chars of output. */
function snapshotWith(count: number, textLen: number, extra?: Partial<Snapshot>): Snapshot {
  const tool_calls: Record<string, ToolCall> = {};
  const timeline: Snapshot["timeline"] = [];
  for (let i = 0; i < count; i++) {
    const id = `t${i}`;
    tool_calls[id] = toolCall(id, textLen);
    timeline.push({ item: "tool_call", id });
  }
  return {
    runs: {},
    timeline,
    tool_calls,
    artifacts: [],
    ...extra,
    provider_incidents: extra?.provider_incidents ?? {},
  };
}

describe("prepareSnapshotForUpload", () => {
  it("passes a small snapshot through untouched (same reference, not elided)", () => {
    const snapshot = snapshotWith(3, 100);
    const prepared = prepareSnapshotForUpload(snapshot);
    expect(prepared.elided).toBe(false);
    expect(prepared.snapshot).toBe(snapshot);
    expect(prepared.json).toBe(JSON.stringify(snapshot));
  });

  it("elides old tool outputs to bring an oversized snapshot under target", () => {
    // 40 tool calls × 50 KB ≈ 2 MB, over a 1 MB target.
    const snapshot = snapshotWith(40, 50_000);
    const prepared = prepareSnapshotForUpload(snapshot, 1_000_000);
    expect(prepared.elided).toBe(true);
    expect(prepared.json.length).toBeLessThan(JSON.stringify(snapshot).length);
    // The original object is never mutated.
    expect(snapshot.tool_calls.t0.content[0]).toEqual({
      type: "text",
      text: "x".repeat(50_000),
    });
  });

  it("keeps the newest tool calls verbatim and drops raw_input on old ones", () => {
    const snapshot = snapshotWith(40, 50_000);
    const prepared = prepareSnapshotForUpload(snapshot, 1_000_000);
    // Newest call is fully intact.
    const newest = prepared.snapshot.tool_calls.t39;
    expect(newest.content[0]).toEqual({ type: "text", text: "x".repeat(50_000) });
    expect(newest.raw_input).toEqual({ path: "t39" });
    // Oldest call is elided to a preview and lost its raw_input.
    const oldest = prepared.snapshot.tool_calls.t0;
    const oldestText = oldest.content[0];
    expect(oldestText.type).toBe("text");
    if (oldestText.type === "text") {
      expect(oldestText.text.length).toBeLessThan(50_000);
      expect(oldestText.text).toContain("elided from history");
    }
    expect(oldest.raw_input).toBeUndefined();
  });

  it("spares at least the most recent KEEP_RECENT_TOOL_CALLS from trimming", () => {
    const snapshot = snapshotWith(40, 50_000);
    const prepared = prepareSnapshotForUpload(snapshot, 1_000_000);
    for (let i = 40 - KEEP_RECENT_TOOL_CALLS; i < 40; i++) {
      const call = prepared.snapshot.tool_calls[`t${i}`];
      expect(call.content[0]).toEqual({ type: "text", text: "x".repeat(50_000) });
    }
  });

  it("elides base64 image data from old tool calls", () => {
    const snapshot = snapshotWith(30, 100);
    // A single huge screenshot in the oldest tool call.
    snapshot.tool_calls.t0.content = [
      { type: "image", mime_type: "image/png", data: "A".repeat(2_000_000) },
    ];
    const prepared = prepareSnapshotForUpload(snapshot, 500_000);
    const block = prepared.snapshot.tool_calls.t0.content[0];
    expect(block.type).toBe("text");
    if (block.type === "text") expect(block.text).toContain("image elided");
  });

  it("never trims user or agent message text", () => {
    const bigMessage = "m".repeat(2_000_000);
    const snapshot = snapshotWith(2, 100, {
      timeline: [
        {
          item: "message",
          run: "r1",
          role: "agent",
          phase: "commentary",
          blocks: [{ type: "text", text: bigMessage }],
        },
        { item: "tool_call", id: "t0" },
        { item: "tool_call", id: "t1" },
      ],
    });
    const prepared = prepareSnapshotForUpload(snapshot, 1_000_000);
    const msg = prepared.snapshot.timeline[0];
    expect(msg.item).toBe("message");
    if (msg.item === "message") {
      expect(msg.blocks[0]).toEqual({ type: "text", text: bigMessage });
      expect(msg.phase).toBe("commentary");
    }
  });

  it("leaves short text blocks (small diffs) alone even when trimming", () => {
    const snapshot = snapshotWith(40, 50_000);
    // A small recent diff-style block should never be touched.
    const small = "diff a.ts\n+one\n-two";
    snapshot.tool_calls.t35.kind = "edit";
    snapshot.tool_calls.t35.content = [{ type: "text", text: small }];
    const prepared = prepareSnapshotForUpload(snapshot, 1_000_000);
    expect(prepared.snapshot.tool_calls.t35.content[0]).toEqual({ type: "text", text: small });
    expect(small.length).toBeLessThan(TEXT_PREVIEW_CHARS);
  });
});
