import { describe, expect, it } from "vitest";
import {
  adjacentConversationId,
  conversationMutationStatusLabel,
  conversationRangeIds,
} from "./sidebarConversationInteractions";

describe("sidebar conversation interactions", () => {
  const ids = ["a", "b", "c", "d"];

  it("selects an inclusive forward or backward range in rendered order", () => {
    expect(conversationRangeIds(ids, "a", "c")).toEqual(["a", "b", "c"]);
    expect(conversationRangeIds(ids, "d", "b")).toEqual(["b", "c", "d"]);
  });

  it("selects only the target when filtering removed the saved anchor", () => {
    expect(conversationRangeIds(["b", "c"], "a", "c")).toEqual(["c"]);
    expect(conversationRangeIds(ids, null, "c")).toEqual(["c"]);
  });

  it("does not make an invisible target selectable", () => {
    expect(conversationRangeIds(ids, "a", "missing")).toEqual([]);
  });

  it("moves keyboard range selection without wrapping around the list", () => {
    expect(adjacentConversationId(ids, "b", 1)).toBe("c");
    expect(adjacentConversationId(ids, "b", -1)).toBe("a");
    expect(adjacentConversationId(ids, "a", -1)).toBeNull();
    expect(adjacentConversationId(ids, "d", 1)).toBeNull();
  });

  it("gives assistive technology concrete mutation progress and outcome", () => {
    expect(conversationMutationStatusLabel({
      id: 1,
      kind: "delete",
      total: 4,
      completed: 1,
      failed: 0,
      pending: 3,
    })).toBe("Deleting 1 of 4 conversations…");
    expect(conversationMutationStatusLabel({
      id: 1,
      kind: "archive",
      total: 1,
      completed: 1,
      failed: 0,
      pending: 0,
    })).toBe("Archived 1 conversation.");
    expect(conversationMutationStatusLabel({
      id: 1,
      kind: "restore",
      total: 2,
      completed: 1,
      failed: 1,
      pending: 0,
    })).toBe("Restored 1 of 2 conversations. 1 failed.");
  });
});
