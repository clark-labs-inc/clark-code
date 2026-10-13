import { beforeEach, describe, expect, it } from "vitest";
import {
  loadConversationProjectBinding,
  removeConversationProjectBinding,
  saveConversationProjectBinding,
} from "./conversationProjectBinding";

describe("conversation project binding", () => {
  beforeEach(() => localStorage.clear());

  it("keeps a chosen path on this device and in this account", () => {
    saveConversationProjectBinding("id:alice", "conversation-1", "/Users/alice/repo");
    expect(loadConversationProjectBinding("id:alice", "conversation-1"))
      .toBe("/Users/alice/repo");
    expect(loadConversationProjectBinding("id:bob", "conversation-1")).toBeNull();
    expect(loadConversationProjectBinding("id:alice", "conversation-2")).toBeNull();
    removeConversationProjectBinding("id:alice", "conversation-1");
    expect(loadConversationProjectBinding("id:alice", "conversation-1")).toBeNull();
  });

  it("does not write a binding without an authenticated account", () => {
    saveConversationProjectBinding(null, "conversation-1", "/repo");
    expect(localStorage.length).toBe(0);
  });
});
