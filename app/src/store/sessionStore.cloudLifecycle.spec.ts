import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot, type Session } from "../core-bridge/types";
import {
  authAccountChanged,
  cloudRequestStillOwned,
  handleCloudConversationDeleted,
  handleCloudHistoryConflict,
} from "./sessionStore.appActions";
import { liveSessions } from "./sessionStore.runtime";
import { useSessionStore } from "./sessionStore";
import { useSpecialistStore } from "./specialistStore";

const bridge = {
  closeSession: vi.fn(async () => {}),
} as unknown as CoreBridge;
const originalOpenConversation = useSessionStore.getState().openConversation;

function conversation(id: string) {
  return {
    id,
    title: `Conversation ${id}`,
    provider: "local",
    createdAt: 1,
    updatedAt: 1,
    rev: 1,
  };
}

beforeEach(() => {
  liveSessions.clear();
  useSpecialistStore.getState().close();
  useSessionStore.setState({
    bridge,
    auth: null,
    session: null,
    snapshot: emptySnapshot(),
    connecting: false,
    opening: null,
    unavailableConversation: null,
    unavailableCleanupId: null,
    conversations: [],
    runningIds: [],
    mutatingConversationIds: new Set(),
    conversationMutation: null,
    warning: null,
    attachments: [],
    historyPrefix: null,
    composerPrefill: null,
    queued: [],
    terminalOpen: false,
    sideQuestion: null,
    activeRemote: null,
    activeRemoteHost: null,
    activeProjectRoot: null,
    openConversation: originalOpenConversation,
  });
});

afterEach(() => {
  vi.clearAllMocks();
  vi.unstubAllGlobals();
  useSpecialistStore.getState().close();
});

describe("cloud conversation index ownership", () => {
  it("keeps the active account visible when native sign-out cannot commit", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const account = {
      user: { id: "account-1", name: "Owner", method: "google" as const },
    };
    invoke.mockRejectedValueOnce(new Error("credential disk unavailable"));
    useSessionStore.setState({ auth: account, error: null });

    await useSessionStore.getState().signOutAuth();

    expect(useSessionStore.getState().auth).toEqual(account);
    expect(useSessionStore.getState().error).toContain("Could not sign out safely");
  });

  it("accepts a valid response after a descriptor refresh for the same stable account", () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const before = {
      user: { id: "account-1", name: "Owner", method: "google" as const },
    };
    const refreshed = { ...before };

    expect(cloudRequestStillOwned(before, refreshed)).toBe(true);
  });

  it("rejects a response after account change", () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const accountOne = {
      user: { id: "account-1", name: "One", method: "google" as const },
    };
    const accountTwo = {
      user: { id: "account-2", name: "Two", method: "google" as const },
    };
    expect(cloudRequestStillOwned(accountOne, accountTwo)).toBe(false);
  });

  it("treats a direct re-auth as an account boundary while preserving a same-user refresh", () => {
    const accountOne = {
      user: { id: "account-1", name: "One", method: "google" as const },
    };
    const refreshed = { ...accountOne };
    const accountTwo = {
      user: { id: "account-2", name: "Two", method: "google" as const },
    };

    expect(authAccountChanged(accountOne, refreshed)).toBe(false);
    expect(authAccountChanged(accountOne, accountTwo)).toBe(true);
  });

  it("does not publish a late prior-account list into the next account's sidebar", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    let resolveList: (value: unknown) => void = () => {
      throw new Error("list resolver was not installed");
    };
    invoke.mockImplementation((command: string) => {
      if (command !== "desktop_conv_list") throw new Error(`unexpected command ${command}`);
      return new Promise((resolve) => {
        resolveList = resolve;
      });
    });
    const accountOne = {
      user: { id: "account-1", name: "One", method: "google" as const },
    };
    const accountTwo = {
      user: { id: "account-2", name: "Two", method: "google" as const },
    };
    useSessionStore.setState({
      auth: accountOne,
      conversations: [conversation("account-one-local")],
      conversationsLoading: true,
    });

    const pendingIndex = useSessionStore.getState().syncCloudIndex();
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledOnce());
    useSessionStore.setState({
      auth: accountTwo,
      conversations: [],
      conversationsLoading: true,
    });
    resolveList([{
      id: "account-one-cloud",
      title: "Account one history",
      provider: "local",
      createdAt: 1,
      updatedAt: 1,
      rev: 1,
    }]);
    await pendingIndex;

    expect(useSessionStore.getState().conversations).toEqual([]);
    expect(useSessionStore.getState().conversationsLoading).toBe(true);
  });
});

describe("other-device conversation lifecycle events", () => {
  it("cancels a deleted opening target and clears state that cannot belong to the next chat", () => {
    useSpecialistStore.getState().open("scout");
    useSessionStore.setState({
      connecting: true,
      opening: {
        id: "deleted",
        kind: "open",
        title: "Deleted",
        remoteHost: null,
      },
      conversations: [conversation("deleted")],
      runningIds: ["deleted"],
      composerPrefill: { text: "stale retry" },
      sideQuestion: {
        sessionId: "deleted",
        question: "old question",
        answer: null,
        error: null,
        loading: true,
        token: 1,
      },
    });

    handleCloudConversationDeleted(
      useSessionStore.setState,
      useSessionStore.getState,
      bridge,
      "deleted",
    );

    const state = useSessionStore.getState();
    expect(state.conversations).toEqual([]);
    expect(state.opening).toBeNull();
    expect(state.connecting).toBe(false);
    expect(state.runningIds).toEqual([]);
    expect(state.composerPrefill).toBeNull();
    expect(state.sideQuestion).toBeNull();
    expect(useSpecialistStore.getState().active).toBeNull();
  });

  it("keeps cleanup ownership mounted when its cloud deletion event arrives first", () => {
    useSessionStore.setState({
      conversations: [conversation("cleanup")],
      unavailableConversation: {
        id: "cleanup",
        title: "Cleanup",
        detail: "missing",
        kind: "unavailable",
      },
      unavailableCleanupId: "cleanup",
    });

    handleCloudConversationDeleted(
      useSessionStore.setState,
      useSessionStore.getState,
      bridge,
      "cleanup",
    );

    expect(useSessionStore.getState()).toMatchObject({
      conversations: [],
      unavailableConversation: {
        id: "cleanup",
        kind: "unavailable",
      },
      unavailableCleanupId: "cleanup",
      warning: null,
    });
  });

  it("turns a conflict during open into a selected refresh state and removes stale activity", () => {
    useSpecialistStore.getState().open("security");
    useSessionStore.setState({
      session: { id: "previous", provider: "local" } as Session,
      connecting: true,
      opening: {
        id: "changed",
        kind: "open",
        title: "Changed",
        remoteHost: null,
      },
      conversations: [conversation("changed")],
      runningIds: ["changed"],
      composerPrefill: { text: "stale retry" },
    });

    handleCloudHistoryConflict(
      useSessionStore.setState,
      useSessionStore.getState,
      bridge,
      "changed",
    );

    const state = useSessionStore.getState();
    expect(state.session).toBeNull();
    expect(state.opening).toBeNull();
    expect(state.runningIds).toEqual([]);
    expect(state.composerPrefill).toBeNull();
    expect(state.unavailableConversation).toEqual({
      id: "changed",
      title: "Conversation changed",
      detail: "Product cloud rejected a stale snapshot revision.",
      kind: "refresh_required",
    });
    expect(useSpecialistStore.getState().active).toBeNull();
  });

  it("does not replace an explicit cleanup with a conflict refresh screen", () => {
    useSessionStore.setState({
      conversations: [conversation("cleanup")],
      unavailableConversation: {
        id: "cleanup",
        title: "Cleanup",
        detail: "missing",
        kind: "unavailable",
      },
      unavailableCleanupId: "cleanup",
      runningIds: ["cleanup"],
    });

    handleCloudHistoryConflict(
      useSessionStore.setState,
      useSessionStore.getState,
      bridge,
      "cleanup",
    );

    expect(useSessionStore.getState()).toMatchObject({
      unavailableConversation: {
        id: "cleanup",
        kind: "unavailable",
      },
      unavailableCleanupId: "cleanup",
      runningIds: [],
    });
  });

  it("keeps an inactive conflict quiet when there is no stale live session", () => {
    useSessionStore.setState({
      conversations: [conversation("background")],
      warning: "Keep this warning",
    });

    handleCloudHistoryConflict(
      useSessionStore.setState,
      useSessionStore.getState,
      bridge,
      "background",
    );

    expect(useSessionStore.getState().warning).toBe("Keep this warning");
    expect(useSessionStore.getState().unavailableConversation).toBeNull();
  });
});
