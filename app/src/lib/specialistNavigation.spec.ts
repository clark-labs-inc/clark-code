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
});
