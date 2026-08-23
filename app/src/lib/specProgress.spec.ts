import { describe, expect, it } from "vitest";

import { emptySnapshot, type Snapshot, type ToolCall } from "../core-bridge/types";
import {
  currentSpecToolCalls,
  specLiveStatus,
  specProgressTitle,
  specRunReceipt,
  specTrailWindow,
} from "./specProgress";

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

  it("returns undefined when the prefix was the whole title", () => {
    // One fallback owner: the ladder decides what to say, not this helper.
    expect(specProgressTitle({ title: "web_fetch: " })).toBeUndefined();
    expect(specProgressTitle({ title: "   " })).toBeUndefined();
  });
});

function busy(over: Partial<Snapshot> = {}): Snapshot {
  return {
    ...emptySnapshot(),
    runs: { run: { id: "run", status: "running" } },
    ...over,
  };
}

/** A turn with one active call, plus whatever the snapshot needs to say so. */
function turn(active: ToolCall, before: ToolCall[] = []): Snapshot {
  const calls = [...before, active];
  return busy({
    tool_calls: Object.fromEntries(calls.map((c) => [c.id, c])),
    timeline: [
      { item: "message", run: "run", role: "user", blocks: [] },
      ...calls.map((c) => ({ item: "tool_call" as const, run: "run", id: c.id })),
    ],
  });
}

describe("specLiveStatus", () => {
  it("prefers reported progress over streamed output over the title", () => {
    const base: ToolCall = {
      id: "a",
      title: "brokered_research: Verifying sources",
      kind: "research",
      status: "in_progress",
      locations: [{ path: "new_SPEC.md" }],
      content: [{ type: "text", text: "• Reading the API reference\n" }],
      progress: {
        revision: 3,
        status: "in_progress",
        latest_activity: "Comparing supported repositories",
        phases: [],
        agents: [],
      },
    };

    const reported = specLiveStatus(turn(base), [base]);
    expect(reported).toMatchObject({
      label: "Comparing supported repositories",
      detail: "new_SPEC.md",
      source: "tool_progress",
    });

    const streamed = { ...base, progress: undefined };
    expect(specLiveStatus(turn(streamed), [streamed])).toMatchObject({
      label: "Reading the API reference",
      source: "tool_stream",
    });

    const titled = { ...streamed, content: [] };
    expect(specLiveStatus(turn(titled), [titled])).toMatchObject({
      label: "Verifying sources",
      source: "tool_title",
    });
  });

  it("clamps a streamed line that is really command output", () => {
    const noisy: ToolCall = {
      id: "a",
      title: "bash: cargo test",
      kind: "execute",
      status: "in_progress",
      locations: [],
      content: [{ type: "text", text: "x".repeat(400) }],
    };

    const status = specLiveStatus(turn(noisy), [noisy]);
    expect(status.source).toBe("tool_stream");
    expect(status.label).toHaveLength(121);
    expect(status.label.endsWith("…")).toBe(true);
  });

  it("never splits an emoji at the clamp boundary", () => {
    const noisy: ToolCall = {
      id: "a",
      title: "bash: seed data",
      kind: "execute",
      status: "in_progress",
      locations: [],
      content: [{ type: "text", text: `${"x".repeat(119)}🚀 trailing` }],
    };

    const label = specLiveStatus(turn(noisy), [noisy]).label;
    // The rocket is a surrogate pair straddling index 120; a naive slice leaves
    // a lone high surrogate that renders as a replacement character.
    expect(label.includes("\ud83d\ude80")).toBe(false);
    expect(label).toBe(`${"x".repeat(119)}…`);
  });

  it("falls through a title that was only a protocol prefix", () => {
    const bare: ToolCall = {
      id: "a",
      title: "web_fetch: ",
      kind: "fetch",
      status: "in_progress",
      locations: [],
      content: [],
    };
    const snapshot = {
      ...turn(bare),
      execution_checklist: {
        revision: 1,
        steps: [{ title: "Draft the recommendation", status: "in_progress" as const }],
      },
    };

    expect(specLiveStatus(snapshot, [bare])).toMatchObject({
      label: "Draft the recommendation",
      source: "checklist",
    });
  });

  it("reports a reply already streaming ahead of the step that preceded it", () => {
    const done = { ...call("a", "grep: Searching sources"), status: "completed" as const };
    const snapshot = busy({
      tool_calls: { a: done },
      timeline: [
        { item: "message", run: "run", role: "user", blocks: [] },
        { item: "tool_call", run: "run", id: "a" },
        { item: "message", run: "run", role: "agent", blocks: [{ type: "text", text: "Here is" }] },
      ],
    });

    expect(specLiveStatus(snapshot, [done])).toMatchObject({
      label: "Writing the spec…",
      source: "drafting",
    });
  });

  it("names a finished step instead of going blank between tool calls", () => {
    // The regression this whole ladder exists for: busy, nothing in flight, and
    // the old code said a bare "Working…".
    const done = { ...call("a", "read_file: Reading the existing spec"), status: "completed" as const };
    const snapshot = busy({
      tool_calls: { a: done },
      timeline: [
        { item: "message", run: "run", role: "user", blocks: [] },
        { item: "tool_call", run: "run", id: "a" },
      ],
    });

    const status = specLiveStatus(snapshot, [done]);
    expect(status).toMatchObject({
      label: "Finished Reading the existing spec",
      source: "last_receipt",
    });
    expect(status.label).not.toBe("Working…");
  });

  it("treats reasoning-only output as thinking", () => {
    const snapshot = busy({
      timeline: [
        { item: "message", run: "run", role: "user", blocks: [] },
        { item: "message", run: "run", role: "agent", blocks: [{ type: "thinking", text: "Weighing options" }] },
      ],
    });

    expect(specLiveStatus(snapshot, [])).toMatchObject({
      label: "Thinking it through…",
      source: "thinking",
    });
  });

  it("reaches a bare Working only when the snapshot carries no evidence", () => {
    const snapshot = busy({
      timeline: [{ item: "artifact", id: "spec" }],
    });

    expect(specLiveStatus(snapshot, [])).toMatchObject({
      label: "Working…",
      source: "unknown",
    });
  });
});

describe("specTrailWindow", () => {
  it("shows everything it can and elides from the head", () => {
    const calls = Array.from({ length: 9 }, (_, i) => call(`c${i}`, `Step ${i}`));

    expect(specTrailWindow(calls.slice(0, 4))).toEqual({ hidden: 0, visible: calls.slice(0, 4) });

    const windowed = specTrailWindow(calls);
    expect(windowed.hidden).toBe(2);
    expect(windowed.visible.map((c) => c.id)).toEqual(["c2", "c3", "c4", "c5", "c6", "c7", "c8"]);
  });

  it("never elides the call that is actually running", () => {
    const calls = Array.from({ length: 9 }, (_, i) => call(`c${i}`, `Step ${i}`));
    calls[0] = { ...calls[0], status: "in_progress" };

    const windowed = specTrailWindow(calls);
    expect(windowed.visible[0].id).toBe("c0");
    expect(windowed.visible).toHaveLength(7);
  });
});

describe("specRunReceipt", () => {
  it("prefers what actually changed on disk", () => {
    const edit: ToolCall = {
      id: "edit",
      title: "Editing the spec",
      kind: "edit",
      status: "completed",
      locations: [{ path: "new_SPEC.md" }],
      content: [{
        type: "text",
        text: "diff --git a/new_SPEC.md b/new_SPEC.md\n@@ -1 +1,2 @@\n context\n+added\n",
      }],
    };

    expect(specRunReceipt([edit])).toEqual({ text: "1 file changed", kind: "edits" });
  });

  it("falls back to the agent's own plan position", () => {
    expect(specRunReceipt([call("a", "Search")], { done: 2, total: 4 }))
      .toEqual({ text: "Step 3 of 4", kind: "steps" });
    expect(specRunReceipt([], { done: 4, total: 4 }))
      .toEqual({ text: "Step 4 of 4", kind: "steps" });
  });

  it("says nothing rather than inventing a number", () => {
    expect(specRunReceipt([call("a", "Search")])).toBeUndefined();
    expect(specRunReceipt([], { done: 0, total: 0 })).toBeUndefined();
  });
});
