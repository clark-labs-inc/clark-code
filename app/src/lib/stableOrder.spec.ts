import { describe, it, expect, beforeEach } from "vitest";
import { stableOrderIds, stableRankMap, __resetStableOrderForTests } from "./stableOrder";

interface Row {
  id: string;
  updatedAt: number;
}
const row = (id: string, updatedAt = 0): Row => ({ id, updatedAt });
const ids = (rows: Row[]) => rows.map((r) => r.id);

beforeEach(() => __resetStableOrderForTests());

describe("stableOrderIds", () => {
  it("keeps first-seen order on the initial render (arrival order preserved)", () => {
    // Store hands conversations newest-first; first render must show them as-is.
    expect(ids(stableOrderIds([row("a"), row("b"), row("c")]))).toEqual(["a", "b", "c"]);
  });

  it("does NOT reorder when a later conversation's updatedAt overtakes an earlier one", () => {
    // The regression: parallel runs bump updatedAt on every flush. After "c"
    // streams, a naive updatedAt-desc sort would float it to the top; stable
    // order must keep it in place.
    stableOrderIds([row("a", 3), row("b", 2), row("c", 1)]); // establish order
    const churned = [row("c", 99), row("a", 3), row("b", 2)]; // store re-prepends "c"
    expect(ids(stableOrderIds(churned))).toEqual(["a", "b", "c"]);
  });

  it("does not reshuffle the rest of the list when one row updates in place", () => {
    stableOrderIds([row("a"), row("b"), row("c"), row("d")]);
    // "b" gets a newer timestamp but the array order is otherwise unchanged.
    const next = [row("a"), row("b", 50), row("c"), row("d")];
    expect(ids(stableOrderIds(next))).toEqual(["a", "b", "c", "d"]);
  });

  it("lands a brand-new conversation on top without disturbing existing rows", () => {
    stableOrderIds([row("a"), row("b")]);
    // A new conversation is prepended by the store (index 0).
    const withNew = [row("new"), row("a"), row("b")];
    expect(ids(stableOrderIds(withNew))).toEqual(["new", "a", "b"]);
  });

  it("drops removed conversations without moving the survivors", () => {
    stableOrderIds([row("a"), row("b"), row("c")]);
    expect(ids(stableOrderIds([row("a"), row("c")]))).toEqual(["a", "c"]);
  });

  it("handles many parallel streams all ticking without any reshuffle", () => {
    const initial = [row("a", 1), row("b", 1), row("c", 1), row("d", 1), row("e", 1)];
    stableOrderIds(initial);
    // Simulate several flushes where each conversation's updatedAt advances and
    // the store re-prepends whichever updated — order must never change.
    let current = initial;
    for (const bump of ["c", "a", "e", "b"]) {
      current = [
        row(bump, Date.now()),
        ...current.filter((r) => r.id !== bump),
      ];
      expect(ids(stableOrderIds(current))).toEqual(["a", "b", "c", "d", "e"]);
    }
  });
});

describe("stableRankMap", () => {
  it("assigns ranks consistent with stableOrderIds", () => {
    const list = [row("a"), row("b"), row("c")];
    stableOrderIds(list);
    const m = stableRankMap(list);
    expect(m.get("a")).toBeLessThan(m.get("b")!);
    expect(m.get("b")).toBeLessThan(m.get("c")!);
  });

  it("keeps a conversation's rank fixed across later calls", () => {
    stableOrderIds([row("a"), row("b")]);
    const before = stableRankMap([row("a"), row("b")]).get("b");
    // "b" overtakes in updatedAt and is re-prepended by the store.
    const after = stableRankMap([row("b", 99), row("a")]).get("b");
    expect(after).toBe(before);
  });
});
