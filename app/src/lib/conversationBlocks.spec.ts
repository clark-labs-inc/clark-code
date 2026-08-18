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
    const result = conversationBlockWindow(timeline, undefined, null, 80);
    expect(result.windowed).toBe(true);
    expect(result.blocks).toHaveLength(80);
    expect(result.rowKeys[0]).toBe("i420");
    expect(result.rowKeys.at(-1)).toBe("i499");
    expect(result).toMatchObject({ start: 420, end: 500, hasEarlier: true, hasLater: false });
  });

  it("bounds a dense tool run by raw item count", () => {
    const timeline = Array.from({ length: 50_000 }, (_, index) => tool(index));
    const result = conversationBlockWindow(timeline, undefined, null, 80);
    expect(result.blocks).toHaveLength(1);
    expect(result.blocks[0]).toMatchObject({
      kind: "work",
      ids: Array.from({ length: 80 }, (_, index) => `tool-${49_920 + index}`),
    });
    expect(result.rowKeys).toEqual(["w49920"]);
  });

  it("pages backward and reports newer history without mounting it", () => {
    const timeline = Array.from({ length: 120 }, (_, index) => message(index));
    const result = conversationBlockWindow(timeline, undefined, 80, 80);
    expect(result.windowed).toBe(true);
    expect(result.blocks).toHaveLength(80);
    expect(result.rowKeys[0]).toBe("i0");
    expect(result.rowKeys.at(-1)).toBe("i79");
    expect(result).toMatchObject({ start: 0, end: 80, hasEarlier: false, hasLater: true });
  });
});
