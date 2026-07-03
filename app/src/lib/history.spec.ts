import { describe, it, expect, beforeEach } from "vitest";
import { setHistoryScope, upsertMeta, loadIndex, deleteConversation } from "./history";
import type { ConversationMeta } from "./history";

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

beforeEach(() => {
  (globalThis as { localStorage: Storage }).localStorage = new MemStorage() as unknown as Storage;
  setHistoryScope("anon");
});

function meta(id: string, title: string): ConversationMeta {
  return { id, title, provider: "local", createdAt: 1, updatedAt: 1 };
}

describe("history account scoping", () => {
  it("isolates conversations between accounts", () => {
    setHistoryScope("alice@example.com");
    upsertMeta(meta("a1", "Alice chat"));
    expect(loadIndex().map((c) => c.id)).toEqual(["a1"]);

    // Switching accounts must NOT reveal the previous account's chats.
    setHistoryScope("bob@example.com");
    expect(loadIndex()).toEqual([]);
    upsertMeta(meta("b1", "Bob chat"));
    expect(loadIndex().map((c) => c.id)).toEqual(["b1"]);

    // Alice's list is intact when she signs back in.
    setHistoryScope("alice@example.com");
    expect(loadIndex().map((c) => c.id)).toEqual(["a1"]);
  });

  it("scope is case-insensitive on the account key", () => {
    setHistoryScope("Alice@Example.com");
    upsertMeta(meta("a1", "Alice"));
    setHistoryScope("alice@example.com");
    expect(loadIndex().map((c) => c.id)).toEqual(["a1"]);
  });

  it("migrates legacy global history into the first account, once", () => {
    const store = localStorage;
    store.setItem("clark.history.index.v1", JSON.stringify([meta("g1", "Legacy")]));
    store.setItem("clark.history.snap.v1.g1", JSON.stringify({ timeline: [] }));

    setHistoryScope("alice@example.com");
    expect(loadIndex().map((c) => c.id)).toEqual(["g1"]);
    // Legacy keys are cleaned up so a second account can't inherit them.
    expect(store.getItem("clark.history.index.v1")).toBeNull();

    setHistoryScope("bob@example.com");
    expect(loadIndex()).toEqual([]);
  });

  it("deletes a conversation from its own scope only", () => {
    setHistoryScope("alice@example.com");
    upsertMeta(meta("a1", "keep"));
    upsertMeta(meta("a2", "remove"));
    deleteConversation("a2");
    expect(loadIndex().map((c) => c.id)).toEqual(["a1"]);
  });
});
