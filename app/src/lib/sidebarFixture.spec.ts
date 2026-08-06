import { describe, expect, it } from "vitest";

import { sidebarFixtureConversations, sidebarFixtureEnabled } from "./sidebarFixture";

describe("sidebar QA fixture", () => {
  it("keeps a realistic, distinct active long list plus archived conversations", () => {
    const conversations = sidebarFixtureConversations(1_700_000_000_000);
    const active = conversations.filter((conversation) => !conversation.archived);
    const archived = conversations.filter((conversation) => conversation.archived);

    expect(active).toHaveLength(24);
    expect(archived).toHaveLength(2);
    expect(new Set(conversations.map((conversation) => conversation.id)).size).toBe(conversations.length);
    expect(new Set(active.map((conversation) => conversation.project)).size).toBe(3);
  });

  it("requires the explicit development QA query parameter", () => {
    expect(sidebarFixtureEnabled("?sidebar-fixture")).toBe(import.meta.env.DEV);
    expect(sidebarFixtureEnabled("?dev")).toBe(false);
  });
});
