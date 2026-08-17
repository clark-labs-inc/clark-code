import { describe, expect, it } from "vitest";

import { emptySnapshot, type Snapshot, type ToolCall } from "../core-bridge/types";
import { currentSpecToolCalls, specProgressTitle } from "./specProgress";

function call(id: string, title: string): ToolCall {
  return {
    id,
    title,
    kind: "search",
    status: "completed",
    locations: [],
    content: [],
  };
}

describe("currentSpecToolCalls", () => {
  it("projects only the latest turn in durable timeline order", () => {
    const snapshot: Snapshot = {
      ...emptySnapshot(),
      tool_calls: {
        old: call("old", "Old search"),
        second: call("second", "Read source"),
        first: call("first", "Search sources"),
      },
      timeline: [
        { item: "message", run: "old-run", role: "user", blocks: [] },
        { item: "tool_call", run: "old-run", id: "old" },
        { item: "message", run: "run", role: "user", blocks: [] },
        { item: "tool_call", run: "run", id: "first" },
        { item: "tool_call", run: "run", id: "second" },
      ],
    };

    expect(currentSpecToolCalls(snapshot).map((item) => item.id)).toEqual(["first", "second"]);
  });
});

describe("specProgressTitle", () => {
  it("hides protocol-shaped tool prefixes from user-facing progress", () => {
    expect(specProgressTitle({ title: "web_fetch: Reading source documentation" })).toBe("Reading source documentation");
    expect(specProgressTitle({ title: "Writing the first draft" })).toBe("Writing the first draft");
  });
});
