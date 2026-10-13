import { describe, expect, it } from "vitest";

import { specialistConversationsForNavigation } from "./specialistNavigation";

describe("specialistConversationsForNavigation", () => {
  it("keeps saved specialist history available to navigation independently of access state", () => {
    const conversations = [{
      id: "missing-security",
      title: "Security audit history",
      provider: "local",
      createdAt: 1,
      updatedAt: 2,
      specialist: { kind: "security" as const },
    }];

    expect(specialistConversationsForNavigation(conversations, "security")).toEqual(
      conversations,
    );
  });

  it("keeps every saved Security conversation reachable after cloud sync", () => {
    const conversations = Array.from({ length: 6 }, (_, index) => ({
      id: `security-${index}`,
      title: `Security conversation ${index}`,
      provider: "local",
      createdAt: index,
      updatedAt: index,
      specialist: { kind: "security" as const },
    }));

    expect(specialistConversationsForNavigation(conversations, "security").map((row) => row.id))
      .toEqual(conversations.map((row) => row.id));
  });
});
