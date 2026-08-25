import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn(async () => undefined));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import type { ConversationMeta } from "../lib/history";
import { liveSessions } from "./sessionStore.runtime";
import { useSessionStore } from "./sessionStore";
import { useSpecialistStore } from "./specialistStore";

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
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
  liveSessions.clear();
  useSpecialistStore.getState().close();
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
  useSpecialistStore.getState().close();
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

  it("paints pending feedback before deletion and between confirmed rows", async () => {
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

    expect(useSessionStore.getState().conversations.map((item) => item.id)).toEqual(["one", "two"]);
    expect(frames).toHaveLength(1);

    frames.shift()?.(0);
    await Promise.resolve();
    expect(frames).toHaveLength(1);
    expect(useSessionStore.getState().conversations.map((item) => item.id)).toEqual(["one", "two"]);

    frames.shift()?.(16);
    await Promise.resolve();
    await Promise.resolve();
    expect(useSessionStore.getState().conversations.map((item) => item.id)).toEqual(["two"]);

    frames.shift()?.(32);
    await Promise.resolve();
    frames.shift()?.(48);
    await Promise.resolve();
    await Promise.resolve();
    expect(useSessionStore.getState().conversations).toEqual([]);

    frames.shift()?.(64);
    await Promise.resolve();
    frames.shift()?.(80);
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

  it("does not steal navigation when a slow restore finishes after the user moves on", async () => {
    let finishRestore!: () => void;
    invoke.mockImplementation(
      () =>
        new Promise<undefined>((resolve) => {
          finishRestore = () => resolve(undefined);
        }),
    );
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const openConversation = vi.fn(async () => {});
    useSessionStore.setState({
      auth: {
        user: { id: "restore-test", name: "Restore test", method: "local" },
      },
      conversations: [conversation("archived", true)],
      openConversation,
    });

    const restoring = useSessionStore.getState().restoreConversation("archived");
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledOnce());
    useSessionStore.getState().endSession();
    finishRestore();
    await restoring;

    expect(useSessionStore.getState().conversations[0]?.archived).toBe(false);
    expect(openConversation).not.toHaveBeenCalled();
    expect(useSessionStore.getState().session).toBeNull();
  });

  it("does not apply a prior account's late archive result to the next account", async () => {
    let finishArchive!: () => void;
    invoke.mockImplementationOnce(
      () => new Promise<undefined>((resolve) => {
        finishArchive = () => resolve(undefined);
      }),
    );
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const accountOne = {
      user: { id: "account-one", name: "One", method: "local" as const },
    };
    const accountTwo = {
      user: { id: "account-two", name: "Two", method: "local" as const },
    };
    useSessionStore.setState({
      auth: accountOne,
      conversations: [conversation("same-id")],
    });

    const archiving = useSessionStore.getState().archiveConversation("same-id");
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledOnce());
    useSessionStore.setState({
      auth: accountTwo,
      conversations: [conversation("same-id")],
      mutatingConversationIds: new Set(),
      conversationMutation: null,
    });
    finishArchive();
    await archiving;

    expect(useSessionStore.getState().conversations).toEqual([conversation("same-id")]);
    expect(useSessionStore.getState().conversationMutation).toBeNull();
  });

  it("deletes an opening specialist conversation and closes its lens", async () => {
    useSpecialistStore.getState().open("scout");
    useSessionStore.setState({
      connecting: true,
      opening: {
        id: "opening",
        kind: "open",
        title: "Opening",
        remoteHost: null,
      },
      conversations: [{
        ...conversation("opening"),
        provider: "specialist",
        specialist: { kind: "scout" },
      }],
      composerPrefill: { text: "stale retry" },
      sideQuestion: {
        sessionId: "opening",
        question: "old question",
        answer: null,
        error: null,
        loading: true,
        token: 1,
      },
    });

    await useSessionStore.getState().deleteConversation("opening");

    expect(useSessionStore.getState()).toMatchObject({
      conversations: [],
      opening: null,
      connecting: false,
      composerPrefill: null,
      sideQuestion: null,
    });
    expect(useSpecialistStore.getState().active).toBeNull();
  });
});
