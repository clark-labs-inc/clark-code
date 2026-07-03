import { describe, it, expect, beforeEach } from "vitest";
import { drainLocalHistory } from "./history";

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
