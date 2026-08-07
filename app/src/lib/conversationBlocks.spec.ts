import { describe, expect, it } from "vitest";
import type { TimelineItem } from "../core-bridge/types";
import { conversationBlockWindow } from "./conversationBlocks";

function message(index: number): TimelineItem {
  return {
    item: "message",
    run: `run-${index}`,
    role: "user",
    blocks: [{ type: "text", text: String(index) }],
  };
}

function tool(index: number, run = "tools"): TimelineItem {
  return { item: "tool_call", run, id: `tool-${index}` };
}

describe("conversationBlockWindow", () => {
  it("groups only the recent suffix while preserving global row identity", () => {
    const timeline = Array.from({ length: 500 }, (_, index) => message(index));
    const result = conversationBlockWindow(timeline, undefined, false, 80);
    expect(result.windowed).toBe(true);
    expect(result.blocks).toHaveLength(80);
    expect(result.rowKeys[0]).toBe("i420");
    expect(result.rowKeys.at(-1)).toBe("i499");
  });

  it("expands the suffix until grouped tool runs still fill the window", () => {
    const timeline = [
      ...Array.from({ length: 200 }, (_, index) => message(index)),
      ...Array.from({ length: 200 }, (_, index) => tool(index)),
    ];
    const result = conversationBlockWindow(timeline, undefined, false, 80);
    expect(result.blocks).toHaveLength(80);
    expect(result.rowKeys.at(-1)).toBe("w200");
  });

  it("returns the complete grouped history when expanded", () => {
    const timeline = Array.from({ length: 120 }, (_, index) => message(index));
    const result = conversationBlockWindow(timeline, undefined, true, 80);
    expect(result.windowed).toBe(false);
    expect(result.blocks).toHaveLength(120);
    expect(result.rowKeys[0]).toBe("i0");
  });
});
