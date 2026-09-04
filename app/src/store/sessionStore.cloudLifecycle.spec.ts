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
import { productRequest } from "../product/productBridge";

const bridge = {
  closeSession: vi.fn(async () => {}),
} as unknown as CoreBridge;
const originalOpenConversation = useSessionStore.getState().openConversation;
const originalSyncCloudIndex = useSessionStore.getState().syncCloudIndex;

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
  invoke.mockReset();
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
    syncCloudIndex: originalSyncCloudIndex,
  });
});

afterEach(() => {
  vi.clearAllMocks();
  vi.unstubAllGlobals();
  useSpecialistStore.getState().close();
});

describe("authenticated product recovery", () => {
  it("refreshes the active native session and replays an expired access check", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const account = {
      user: { id: "account-1", name: "Owner", method: "google" as const },
    };
    const refreshed = { ...account, connection: "ready" as const };
    let accessAttempts = 0;
    invoke.mockImplementation((command: string, args?: { operation?: string }) => {
      if (command !== "product_request") throw new Error(`unexpected command ${command}`);
      if (args?.operation === "account.refresh") return Promise.resolve(refreshed);
      if (args?.operation === "access.snapshot" && accessAttempts++ === 0) {
        return Promise.reject(new Error("401 ExpiredSignature"));
      }
      if (args?.operation === "access.snapshot") return Promise.resolve({ schema_version: 1 });
      throw new Error(`unexpected operation ${args?.operation}`);
    });
    useSessionStore.setState({ auth: account, warning: null });

    await expect(productRequest("access.snapshot")).resolves.toEqual({ schema_version: 1 });

    expect(invoke.mock.calls.map(([, args]) => args?.operation)).toEqual([
      "access.snapshot",
      "account.refresh",
      "access.snapshot",
    ]);
    expect(useSessionStore.getState().auth).toEqual(refreshed);
    expect(useSessionStore.getState().warning).toBeNull();
  });

  it("marks the account reconnectable and does not replay when refresh fails", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const account = {
      user: { id: "account-1", name: "Owner", method: "google" as const },
    };
    invoke.mockImplementation((command: string, args?: { operation?: string }) => {
      if (command !== "product_request") throw new Error(`unexpected command ${command}`);
      if (args?.operation === "access.snapshot") {
        return Promise.reject(new Error("401 ExpiredSignature"));
      }
      if (args?.operation === "account.refresh") {
        return Promise.reject(new Error("refresh revoked"));
      }
      throw new Error(`unexpected operation ${args?.operation}`);
    });
    useSessionStore.setState({ auth: account, warning: null });

    await expect(productRequest("access.snapshot")).rejects.toThrow("refresh revoked");

    expect(invoke.mock.calls.map(([, args]) => args?.operation)).toEqual([
      "access.snapshot",
      "account.refresh",
    ]);
    expect(useSessionStore.getState().auth?.connection).toBe("reconnect_required");
    expect(useSessionStore.getState().warning).toContain("needs reconnecting");
  });

  it("does not publish or replay a refresh after the active account changes", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const accountOne = {
      user: { id: "account-1", name: "One", method: "google" as const },
    };
    const accountTwo = {
      user: { id: "account-2", name: "Two", method: "google" as const },
    };
    let releaseRefresh = () => {};
    invoke.mockImplementation((command: string, args?: { operation?: string }) => {
      if (command !== "product_request") throw new Error(`unexpected command ${command}`);
      if (args?.operation === "access.snapshot") {
        return Promise.reject(new Error("401 ExpiredSignature"));
      }
      if (args?.operation === "account.refresh") {
        return new Promise((resolve) => {
          releaseRefresh = () => resolve(accountOne);
        });
      }
      throw new Error(`unexpected operation ${args?.operation}`);
    });
    useSessionStore.setState({ auth: accountOne, warning: null });

    const access = productRequest("access.snapshot");
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
    useSessionStore.setState({ auth: accountTwo });
    releaseRefresh();

    await expect(access).rejects.toThrow("active account changed");
    expect(invoke.mock.calls.map(([, args]) => args?.operation)).toEqual([
      "access.snapshot",
      "account.refresh",
    ]);
    expect(useSessionStore.getState().auth).toEqual(accountTwo);
    expect(useSessionStore.getState().warning).toBeNull();
  });

  it("manually reconnects through the shared refresh boundary before syncing", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const account = {
      user: { id: "account-1", name: "Owner", method: "google" as const },
      connection: "reconnect_required" as const,
    };
    const refreshed = { ...account, connection: "ready" as const };
    const syncCloudIndex = vi.fn(async () => {});
    invoke.mockImplementation((command: string, args?: { operation?: string }) => {
      if (command !== "product_request") throw new Error(`unexpected command ${command}`);
      if (args?.operation === "account.refresh") return Promise.resolve(refreshed);
      throw new Error(`unexpected operation ${args?.operation}`);
    });
    useSessionStore.setState({
      auth: account,
      error: "prior error",
      warning: "Your account needs reconnecting.",
      syncCloudIndex,
    });

    await useSessionStore.getState().reconnectAuth();

    expect(invoke.mock.calls.map(([, args]) => args?.operation)).toEqual([
      "account.refresh",
    ]);
    expect(useSessionStore.getState().auth).toEqual(refreshed);
    expect(useSessionStore.getState().error).toBeNull();
    expect(useSessionStore.getState().warning).toBeNull();
    expect(syncCloudIndex).toHaveBeenCalledOnce();
  });
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
