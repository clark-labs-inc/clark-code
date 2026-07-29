import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ConversationMeta } from "../lib/history";
import { liveSessions } from "./sessionStore.runtime";
import { useSessionStore } from "./sessionStore";

const originalOpenConversation = useSessionStore.getState().openConversation;

function conversation(id: string, archived = false): ConversationMeta {
  return {
    id,
    title: `Conversation ${id}`,
    provider: "local",
    project: "/tmp/sidebar-fixture",
    createdAt: 1,
    updatedAt: 1,
    archived,
  };
}

beforeEach(() => {
  liveSessions.clear();
  useSessionStore.setState({
    auth: null,
    bridge: null,
    session: null,
    opening: null,
    conversations: [],
    runningIds: [],
    selectedConversationIds: new Set(),
    mutatingConversationIds: new Set(),
    conversationMutation: null,
    error: null,
    openConversation: originalOpenConversation,
  });
});

afterEach(() => {
  vi.clearAllTimers();
  vi.unstubAllGlobals();
  vi.useRealTimers();
  useSessionStore.setState({ openConversation: originalOpenConversation });
});

describe("sidebar conversation mutations", () => {
  it("archives selected conversations as each durable operation completes", async () => {
    useSessionStore.setState({
      conversations: [conversation("one"), conversation("two"), conversation("three")],
      selectedConversationIds: new Set(["one", "two", "three"]),
    });
    const activeAfterEachConfirmation: string[][] = [];
    const unsubscribe = useSessionStore.subscribe((state) => {
      if (state.conversationMutation?.kind === "archive" && state.conversationMutation.completed > 0) {
        activeAfterEachConfirmation.push(
          state.conversations.filter((item) => !item.archived).map((item) => item.id),
        );
      }
    });

    await useSessionStore.getState().archiveSelectedConversations();
    unsubscribe();

    expect(activeAfterEachConfirmation).toContainEqual(["two", "three"]);
    expect(activeAfterEachConfirmation).toContainEqual(["three"]);
    expect(useSessionStore.getState().conversations.every((item) => item.archived)).toBe(true);
    expect(useSessionStore.getState().selectedConversationIds).toEqual(new Set());
    expect(useSessionStore.getState().conversationMutation).toMatchObject({
      kind: "archive",
      completed: 3,
      pending: 0,
    });
  });

  it("deletes selected conversations one visible row at a time", async () => {
    useSessionStore.setState({
      conversations: [conversation("one"), conversation("two"), conversation("three")],
      selectedConversationIds: new Set(["one", "two", "three"]),
    });
    const remainingAfterEachConfirmation: string[][] = [];
    const unsubscribe = useSessionStore.subscribe((state) => {
      if (state.conversationMutation?.kind === "delete" && state.conversationMutation.completed > 0) {
        remainingAfterEachConfirmation.push(state.conversations.map((item) => item.id));
      }
    });

    await useSessionStore.getState().deleteSelectedConversations();
    unsubscribe();

    expect(remainingAfterEachConfirmation).toContainEqual(["two", "three"]);
    expect(remainingAfterEachConfirmation).toContainEqual(["three"]);
    expect(useSessionStore.getState().conversations).toEqual([]);
    expect(useSessionStore.getState().conversationMutation).toMatchObject({
      kind: "delete",
      completed: 3,
      pending: 0,
    });
  });

  it("waits for two browser frames between confirmed deletion rows", async () => {
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      frames.push(callback);
      return frames.length;
    });
    useSessionStore.setState({
      conversations: [conversation("one"), conversation("two")],
      selectedConversationIds: new Set(["one", "two"]),
    });

    const deleting = useSessionStore.getState().deleteSelectedConversations();
    await Promise.resolve();

    expect(useSessionStore.getState().conversations.map((item) => item.id)).toEqual(["two"]);
    expect(frames).toHaveLength(1);

    frames.shift()?.(0);
    await Promise.resolve();
    expect(frames).toHaveLength(1);
    expect(useSessionStore.getState().conversations.map((item) => item.id)).toEqual(["two"]);

    frames.shift()?.(16);
    await Promise.resolve();
    await Promise.resolve();
    expect(useSessionStore.getState().conversations).toEqual([]);

    frames.shift()?.(32);
    await Promise.resolve();
    frames.shift()?.(48);
    await deleting;
  });

  it("restores the conversation and immediately hands it to the open flow", async () => {
    const openConversation = vi.fn(async () => {});
    useSessionStore.setState({
      conversations: [conversation("archived", true)],
      openConversation,
    });

    await useSessionStore.getState().restoreConversation("archived");

    expect(useSessionStore.getState().conversations[0]?.archived).toBe(false);
    expect(openConversation).toHaveBeenCalledWith("archived");
    expect(useSessionStore.getState().conversationMutation).toMatchObject({
      kind: "restore",
      completed: 1,
      pending: 0,
    });
  });
});
