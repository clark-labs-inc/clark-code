import { describe, expect, it } from "vitest";
import type { Snapshot, ToolCall } from "../core-bridge/types";
import { prepareSnapshotForUpload, utf8ByteLength } from "./snapshotUpload";

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

function snapshotWith(count: number, textLen: number): Snapshot {
  const tool_calls: Record<string, ToolCall> = {};
  const timeline: Snapshot["timeline"] = [];
  for (let index = 0; index < count; index++) {
    const id = `t${index}`;
    tool_calls[id] = toolCall(id, textLen);
    timeline.push({ item: "tool_call", id });
  }
  return { runs: {}, timeline, tool_calls, artifacts: [], provider_incidents: {} };
}

describe("prepareSnapshotForUpload", () => {
  it("passes a small snapshot through byte-for-byte", () => {
    const snapshot = snapshotWith(3, 100);
    const prepared = prepareSnapshotForUpload(snapshot);
    expect(prepared.snapshot).toBe(snapshot);
    expect(prepared.json).toBe(JSON.stringify(snapshot));
    expect(prepared.bytes).toBe(utf8ByteLength(prepared.json));
  });

  it("preserves every oversized tool output and raw input byte-for-byte", () => {
    const snapshot = snapshotWith(40, 50_000);
    const prepared = prepareSnapshotForUpload(snapshot);
    expect(prepared.snapshot).toBe(snapshot);
    expect(prepared.json).toBe(JSON.stringify(snapshot));
    expect(prepared.snapshot.tool_calls.t0.content[0]).toEqual({
      type: "text",
      text: "x".repeat(50_000),
    });
    expect(prepared.snapshot.tool_calls.t0.raw_input).toEqual({ path: "t0" });
    expect(prepared.snapshot.tool_calls.t39.content[0]).toEqual({
      type: "text",
      text: "x".repeat(50_000),
    });
  });

  it("preserves typed image and audio bytes", () => {
    const snapshot = snapshotWith(2, 10);
    snapshot.tool_calls.t0.content = [
      { type: "image", mime_type: "image/png", data: "IMAGE_BYTES" },
    ];
    snapshot.tool_calls.t1.content = [
      { type: "audio", mime_type: "audio/wav", data: "AUDIO_BYTES" },
    ];
    const roundTrip = JSON.parse(prepareSnapshotForUpload(snapshot).json) as Snapshot;
    expect(roundTrip.tool_calls.t0.content).toEqual(snapshot.tool_calls.t0.content);
    expect(roundTrip.tool_calls.t1.content).toEqual(snapshot.tool_calls.t1.content);
  });

  it("measures serialized limits in UTF-8 bytes, not UTF-16 code units", () => {
    const snapshot = snapshotWith(0, 0);
    snapshot.timeline.push({
      item: "message",
      run: "r1",
      role: "user",
      blocks: [{ type: "text", text: "😀".repeat(100) }],
    });
    const prepared = prepareSnapshotForUpload(snapshot);
    expect(prepared.json.length).toBeLessThan(prepared.bytes);
    expect(prepared.bytes).toBe(utf8ByteLength(prepared.json));
  });
});
