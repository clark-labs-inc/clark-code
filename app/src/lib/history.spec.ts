import { describe, it, expect, beforeEach } from "vitest";
import { drainLocalHistory, renderResumeContext, settleRuns } from "./history";
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

describe("renderResumeContext", () => {
  it("renders messages and tool lines, stripping thinking spans", () => {
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
    const out = renderResumeContext(snapshot)!;
    expect(out).toContain("User: install node");
    expect(out).toContain("[execute] brew install node — /tmp (completed)");
    expect(out).toContain("Assistant: Waiting for brew to finish.");
    expect(out).not.toContain("hmm");
  });

  it("returns null for an empty transcript and keeps the tail when over budget", () => {
    expect(
      renderResumeContext({ runs: {}, timeline: [], tool_calls: {}, artifacts: [] }),
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
    const out = renderResumeContext(long, 500)!;
    expect(out.length).toBeLessThan(600);
    expect(out).toContain("(earlier history truncated)");
    expect(out).toContain("turn 49");
    expect(out).not.toContain("turn 0 ");
  });
});
