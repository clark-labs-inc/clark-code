import { describe, expect, it } from "vitest";

import type { ConversationMeta } from "../lib/history";
import { mergeConversations } from "./sessionStore.runtime";

function conversation(id: string, rev?: number): ConversationMeta {
  return {
    id,
    title: `Conversation ${id}`,
    provider: "local",
    createdAt: 1,
    updatedAt: 1,
    rev,
  };
}

describe("cloud conversation index reconciliation", () => {
  it("does not resurrect a revisioned local row missing from the authoritative cloud list", () => {
    expect(mergeConversations([], [conversation("deleted-elsewhere", 3)])).toEqual([]);
  });

  it("keeps a not-yet-synced local row while accepting cloud metadata as authoritative", () => {
    const pending = conversation("pending-local");
    const stale = { ...conversation("shared", 1), title: "Stale local title" };
    const current = { ...conversation("shared", 2), title: "Current cloud title" };

    expect(mergeConversations([current], [pending, stale])).toEqual([pending, current]);
  });
});
