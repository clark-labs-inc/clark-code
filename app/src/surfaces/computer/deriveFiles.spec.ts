import { describe, expect, it } from "vitest";
import { deriveTouchedFiles, focusedPath } from "./deriveFiles";
import { emptySnapshot, type Snapshot, type ToolCall } from "../../core-bridge/types";

function tc(over: Partial<ToolCall> & { id: string }): ToolCall {
  return {
    title: "t",
    kind: "read",
    status: "completed",
    locations: [],
    content: [],
    ...over,
  };
}

function snap(calls: ToolCall[]): Snapshot {
  const s = emptySnapshot();
  for (const c of calls) s.tool_calls[c.id] = c;
  return s;
}

describe("deriveTouchedFiles", () => {
  it("returns one entry per touched path with latest content", () => {
    const s = snap([
      tc({
        id: "a",
        kind: "read",
        locations: [{ path: "src/main.rs" }],
        content: [{ type: "text", text: "old" }],
      }),
      tc({
        id: "b",
        kind: "edit",
        status: "completed",
        locations: [{ path: "src/main.rs" }],
        content: [{ type: "text", text: "diff src/main.rs\n+new" }],
      }),
      tc({ id: "c", kind: "read", locations: [{ path: "Cargo.toml" }] }),
    ]);
    const files = deriveTouchedFiles(s);
    expect(files.map((f) => f.path)).toEqual(["src/main.rs", "Cargo.toml"]);
    const main = files[0];
    expect(main.kind).toBe("edit"); // latest touch wins
    expect(main.isDiff).toBe(true);
  });

  it("ignores tool calls without a file location", () => {
    const s = snap([tc({ id: "x", kind: "execute", locations: [] })]);
    expect(deriveTouchedFiles(s)).toHaveLength(0);
  });

  it("focusedPath reflects a files-surface focus", () => {
    const s = emptySnapshot();
    s.focus = { surface: "files", path: "a/b.ts" };
    expect(focusedPath(s)).toBe("a/b.ts");
    s.focus = { surface: "terminal" };
    expect(focusedPath(s)).toBeUndefined();
  });
});
