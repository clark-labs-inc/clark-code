import { describe, it, expect, beforeEach } from "vitest";
import {
  stableOrderIds,
  stableProjectOrder,
  stableRankMap,
  __resetStableOrderForTests,
} from "./stableOrder";

interface Row {
  id: string;
  createdAt: number;
  updatedAt: number;
}
const row = (id: string, createdAt = 0, updatedAt = 0): Row => ({ id, createdAt, updatedAt });
const ids = (rows: Row[]) => rows.map((r) => r.id);

beforeEach(() => __resetStableOrderForTests());

describe("stableOrderIds", () => {
  it("puts the latest-created conversation first regardless of source order", () => {
    expect(ids(stableOrderIds([row("old", 1), row("new", 3), row("middle", 2)]))).toEqual([
      "new",
      "middle",
      "old",
    ]);
  });

  it("does NOT reorder when a later conversation's updatedAt overtakes an earlier one", () => {
    // The regression: parallel runs bump updatedAt on every flush. After "c"
    // streams, a naive updatedAt-desc sort would float it to the top; stable
    // order must keep it in place.
    stableOrderIds([row("a", 3, 3), row("b", 2, 2), row("c", 1, 1)]);
    const churned = [row("c", 1, 99), row("a", 3, 3), row("b", 2, 2)];
    expect(ids(stableOrderIds(churned))).toEqual(["a", "b", "c"]);
  });

  it("does not reshuffle the rest of the list when one row updates in place", () => {
    stableOrderIds([row("a", 4), row("b", 3), row("c", 2), row("d", 1)]);
    // "b" gets a newer timestamp but the array order is otherwise unchanged.
    const next = [row("a", 4), row("b", 3, 50), row("c", 2), row("d", 1)];
    expect(ids(stableOrderIds(next))).toEqual(["a", "b", "c", "d"]);
  });

  it("does not reshuffle conversations created in the same millisecond", () => {
    expect(ids(stableOrderIds([row("a", 1), row("b", 1)]))).toEqual(["a", "b"]);
    expect(ids(stableOrderIds([row("b", 1, 50), row("a", 1)]))).toEqual(["a", "b"]);
  });

  it("lands a brand-new conversation on top without disturbing existing rows", () => {
    stableOrderIds([row("a", 2), row("b", 1)]);
    // A new conversation is prepended by the store (index 0).
    const withNew = [row("new", 3), row("a", 2), row("b", 1)];
    expect(ids(stableOrderIds(withNew))).toEqual(["new", "a", "b"]);
  });

  it("drops removed conversations without moving the survivors", () => {
    stableOrderIds([row("a", 3), row("b", 2), row("c", 1)]);
    expect(ids(stableOrderIds([row("a", 3), row("c", 1)]))).toEqual(["a", "c"]);
  });

  it("handles many parallel streams all ticking without any reshuffle", () => {
    const initial = [row("a", 5), row("b", 4), row("c", 3), row("d", 2), row("e", 1)];
    stableOrderIds(initial);
    // Simulate several flushes where each conversation's updatedAt advances and
    // the store re-prepends whichever updated — order must never change.
    let current = initial;
    for (const bump of ["c", "a", "e", "b"]) {
      current = [
        row(bump, current.find((item) => item.id === bump)!.createdAt, Date.now()),
        ...current.filter((r) => r.id !== bump),
      ];
      expect(ids(stableOrderIds(current))).toEqual(["a", "b", "c", "d", "e"]);
    }
  });
});

describe("stableRankMap", () => {
  it("assigns ranks consistent with stableOrderIds", () => {
    const list = [row("a", 3), row("b", 2), row("c", 1)];
    stableOrderIds(list);
    const m = stableRankMap(list);
    expect(m.get("a")).toBeLessThan(m.get("b")!);
    expect(m.get("b")).toBeLessThan(m.get("c")!);
  });

  it("keeps a conversation's rank fixed across later calls", () => {
    stableOrderIds([row("a", 2), row("b", 1)]);
    const before = stableRankMap([row("a", 2), row("b", 1)]).get("b");
    // "b" overtakes in updatedAt and is re-prepended by the store.
    const after = stableRankMap([row("b", 1, 99), row("a", 2)]).get("b");
    expect(after).toBe(before);
  });
});

describe("stableProjectOrder", () => {
  const project = (key: string) => ({ key });

  it("does not move a project when its remaining conversation changes its derived rank", () => {
    stableProjectOrder([project("project-a"), project("project-b"), project("project-c")]);

    expect(
      stableProjectOrder([project("project-b"), project("project-a"), project("project-c")])
        .map((item) => item.key),
    ).toEqual(["project-a", "project-b", "project-c"]);
  });

  it("keeps a temporarily empty project in its original slot", () => {
    stableProjectOrder([project("project-a"), project("project-b"), project("project-c")]);
    stableProjectOrder([project("project-a"), project("project-c")]);

    expect(
      stableProjectOrder([project("project-a"), project("project-b"), project("project-c")])
        .map((item) => item.key),
    ).toEqual(["project-a", "project-b", "project-c"]);
  });

  it("places a genuinely new project above existing projects", () => {
    stableProjectOrder([project("project-a"), project("project-b")]);

    expect(
      stableProjectOrder([project("project-new"), project("project-a"), project("project-b")])
        .map((item) => item.key),
    ).toEqual(["project-new", "project-a", "project-b"]);
  });

  it("allows pinned-project priority to override stable position", () => {
    const initial = [project("project-a"), project("project-b")];
    stableProjectOrder(initial);

    expect(
      stableProjectOrder(initial, (item) => item.key === "project-b" ? 0 : 1)
        .map((item) => item.key),
    ).toEqual(["project-b", "project-a"]);
  });
});
