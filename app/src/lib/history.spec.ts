import { describe, it, expect, beforeEach } from "vitest";
import {
  buildResumeTranscript,
  drainLocalHistory,
  migratePlanningSnapshot,
  settleRuns,
  snapshotBeforeTimelineItem,
} from "./history";
import type { Snapshot, ToolCall } from "../core-bridge/types";

// The Node test env has no localStorage; back it with a tiny in-memory mock.
class MemStorage {
  private m = new Map<string, string>();
  get length() {
    return this.m.size;
  }
  key(i: number) {
    return [...this.m.keys()][i] ?? null;
  }
  getItem(k: string) {
    return this.m.has(k) ? this.m.get(k)! : null;
  }
  setItem(k: string, v: string) {
    this.m.set(k, String(v));
  }
  removeItem(k: string) {
    this.m.delete(k);
  }
  clear() {
    this.m.clear();
  }
}

let store: MemStorage;
beforeEach(() => {
  store = new MemStorage();
  (globalThis as { localStorage: Storage }).localStorage = store as unknown as Storage;
});

function meta(id: string, title: string, archived?: boolean) {
  return { id, title, provider: "local", createdAt: 1, updatedAt: 1, ...(archived ? { archived } : {}) };
}
function snap(id: string) {
  return { timeline: [{ item: "message", run: id, role: "user", blocks: [] }], runs: {}, tool_calls: {}, artifacts: [] };
}

describe("drainLocalHistory", () => {
  it("returns nothing and no-ops when there is no local history", () => {
    expect(drainLocalHistory()).toEqual([]);
  });

  it("drains legacy global keys and deletes them", () => {
    store.setItem("clark.history.index.v1", JSON.stringify([meta("g1", "Legacy")]));
    store.setItem("clark.history.snap.v1.g1", JSON.stringify(snap("g1")));

    const drained = drainLocalHistory();
    expect(drained.map((d) => d.meta.id)).toEqual(["g1"]);
    expect(drained[0].snapshot.timeline.length).toBe(1);
    // Keys are removed so a later launch finds nothing.
    expect(store.getItem("clark.history.index.v1")).toBeNull();
    expect(store.getItem("clark.history.snap.v1.g1")).toBeNull();
    expect(drainLocalHistory()).toEqual([]);
  });

  it("drains per-account scoped keys across accounts and carries archived", () => {
    store.setItem("clark.history.index.v1.alice@x.com", JSON.stringify([meta("a1", "Alice", true)]));
    store.setItem("clark.history.snap.v1.alice@x.com.a1", JSON.stringify(snap("a1")));
    store.setItem("clark.history.index.v1.bob@x.com", JSON.stringify([meta("b1", "Bob")]));
    store.setItem("clark.history.snap.v1.bob@x.com.b1", JSON.stringify(snap("b1")));

    const drained = drainLocalHistory();
    const byId = Object.fromEntries(drained.map((d) => [d.meta.id, d]));
    expect(Object.keys(byId).sort()).toEqual(["a1", "b1"]);
    expect(byId.a1.archived).toBe(true);
    expect(byId.b1.archived).toBe(false);
    // Everything removed.
    expect(store.length).toBe(0);
  });

  it("ignores index entries whose snapshot is missing", () => {
    store.setItem("clark.history.index.v1", JSON.stringify([meta("g1", "no snap")]));
    const drained = drainLocalHistory();
    expect(drained).toEqual([]);
    expect(store.getItem("clark.history.index.v1")).toBeNull();
  });
});

function tool(id: string, status: ToolCall["status"]): ToolCall {
  return { id, title: id, kind: "execute", status, locations: [], content: [] };
}

describe("settleRuns", () => {
  it("cancels live runs and settles in-flight tool calls quietly", () => {
    const snapshot: Snapshot = {
      runs: {
        r1: { id: "r1", status: "running" },
        r2: { id: "r2", status: "done" },
      },
      timeline: [],
      tool_calls: {
        t1: tool("t1", "in_progress"),
        t2: tool("t2", "pending"),
        t3: tool("t3", "failed"),
      },
      artifacts: [],
      pending_permission: { id: "p1", session: "s1", title: "allow?", options: [] },
    };
    const settled = settleRuns(snapshot);
    expect(settled.runs.r1.status).toBe("cancelled");
    expect(settled.runs.r2.status).toBe("done");
    // Interrupted tools settle to completed (no glyph), never left spinning
    // and never coerced to failed; real failures are preserved.
    expect(settled.tool_calls.t1.status).toBe("completed");
    expect(settled.tool_calls.t2.status).toBe("completed");
    expect(settled.tool_calls.t3.status).toBe("failed");
    expect(settled.pending_permission).toBeUndefined();
  });

  it("returns the snapshot unchanged when nothing is live", () => {
    const snapshot: Snapshot = {
      runs: { r1: { id: "r1", status: "done" } },
      timeline: [],
      tool_calls: { t1: tool("t1", "completed") },
      artifacts: [],
    };
    expect(settleRuns(snapshot)).toBe(snapshot);
  });
});

describe("buildResumeTranscript", () => {
  it("preserves typed messages and tools while stripping thinking spans", () => {
    const snapshot: Snapshot = {
      runs: {},
      timeline: [
        { item: "message", run: "user", role: "user", blocks: [{ type: "text", text: "install node" }] },
        { item: "tool_call", id: "t1" },
        {
          item: "message",
          run: "r1",
          role: "agent",
          blocks: [{ type: "text", text: "<thinking>hmm</thinking>Waiting for brew to finish." }],
        },
      ],
      tool_calls: { t1: { ...tool("t1", "completed"), title: "brew install node", locations: [{ path: "/tmp" }] } },
      artifacts: [],
    };
    const out = buildResumeTranscript(snapshot)!;
    expect(out.items[0]).toMatchObject({ item: "message", role: "user" });
    expect(out.items[1]).toMatchObject({
      item: "tool_call",
      title: "brew install node",
      kind: "execute",
      status: "completed",
      locations: [{ path: "/tmp" }],
    });
    expect(JSON.stringify(out)).toContain("Waiting for brew to finish.");
    expect(JSON.stringify(out)).not.toContain("hmm");
  });

  it("returns null for an empty transcript and keeps the tail when over budget", () => {
    expect(
      buildResumeTranscript({ runs: {}, timeline: [], tool_calls: {}, artifacts: [] }),
    ).toBeNull();

    const long: Snapshot = {
      runs: {},
      timeline: Array.from({ length: 50 }, (_, i) => ({
        item: "message" as const,
        run: "user",
        role: "user" as const,
        blocks: [{ type: "text" as const, text: `turn ${i} ${"x".repeat(40)}` }],
      })),
      tool_calls: {},
      artifacts: [],
    };
    const out = buildResumeTranscript(long, 500)!;
    expect(JSON.stringify(out).length).toBeLessThan(700);
    expect(out.truncated).toBe(true);
    expect(JSON.stringify(out)).toContain("turn 49");
    expect(JSON.stringify(out)).not.toContain("turn 0 ");
  });

  it("replays the latest proposed plan as typed state", () => {
    const snapshot: Snapshot = {
      runs: {}, timeline: [], tool_calls: {}, artifacts: [],
      proposed_plan: {
        id: "plan-1", revision: 2, markdown: "1. Build it", status: "awaiting_decision",
      },
    };
    expect(buildResumeTranscript(snapshot)?.items).toContainEqual({
      item: "proposed_plan",
      plan: snapshot.proposed_plan,
    });
  });
});

describe("planning history migration", () => {
  it("upgrades the old overloaded plan shape once at the replay boundary", () => {
    const legacy = {
      runs: {}, tool_calls: {}, artifacts: [],
      plan: { phases: [{ title: "Inspect", status: "in_progress" }] },
      timeline: [{
        item: "plan", run: "r1",
        plan: { phases: [{ title: "Inspect", status: "in_progress" }] },
      }],
    } as unknown as Snapshot;
    const migrated = migratePlanningSnapshot(legacy);
    expect(migrated.execution_checklist?.steps[0].title).toBe("Inspect");
    expect(migrated.timeline[0]).toMatchObject({ item: "execution_checklist", run: "r1" });
    expect("plan" in migrated).toBe(false);
  });
});

describe("snapshotBeforeTimelineItem", () => {
  it("drops the edited turn and every later branch item", () => {
    const snapshot: Snapshot = {
      session: "chat-1",
      runs: {
        r1: { id: "r1", status: "done" },
        r2: { id: "r2", status: "failed" },
      },
      timeline: [
        { item: "message", run: "user", role: "user", blocks: [{ type: "text", text: "first" }] },
        { item: "tool_call", id: "t1" },
        { item: "message", run: "r1", role: "agent", blocks: [{ type: "text", text: "done" }] },
        { item: "artifact", id: "a1" },
        {
          item: "message",
          run: "user",
          role: "user",
          blocks: [{ type: "text", text: "old second" }],
        },
        { item: "tool_call", id: "t2" },
        { item: "message", run: "r2", role: "agent", blocks: [{ type: "text", text: "failed" }] },
        { item: "artifact", id: "a2" },
      ],
      tool_calls: {
        t1: tool("t1", "completed"),
        t2: tool("t2", "failed"),
      },
      artifacts: [
        { id: "a1", title: "kept", kind: "file" },
        { id: "a2", title: "dropped", kind: "file" },
      ],
      pending_permission: { id: "p1", session: "chat-1", title: "stale", options: [] },
      focus: { surface: "files", path: "stale.ts" },
      fan_out: { title: "stale", total: 1, done: 0, running: 1, agents: [] },
    };

    const prefix = snapshotBeforeTimelineItem(snapshot, 4);

    expect(prefix.timeline).toHaveLength(4);
    expect(
      prefix.timeline.some(
        (item) =>
          item.item === "message" &&
          item.role === "user" &&
          item.blocks.some(
            (block) => block.type === "text" && block.text === "old second",
          ),
      ),
    ).toBe(false);
    expect(Object.keys(prefix.runs)).toEqual(["r1"]);
    expect(Object.keys(prefix.tool_calls)).toEqual(["t1"]);
    expect(prefix.artifacts.map((artifact) => artifact.id)).toEqual(["a1"]);
    expect(prefix.pending_permission).toBeUndefined();
    expect(prefix.focus).toBeUndefined();
    expect(prefix.fan_out).toBeUndefined();
  });
});
