import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot, type Session } from "../core-bridge/types";
import { liveSessions } from "./sessionStore.runtime";
import { useSessionStore } from "./sessionStore";
import { useSpecialistStore } from "./specialistStore";
import { productRequest } from "../product/productBridge";

const bridge = {
  closeSession: vi.fn(async () => {}),
} as unknown as CoreBridge;
const originalOpenConversation = useSessionStore.getState().openConversation;
const originalSyncCloudIndex = useSessionStore.getState().syncCloudIndex;

beforeEach(() => {
  invoke.mockReset();
  useSessionStore.setState({ authReconnecting: false, authReconnectError: null });
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
        return Promise.reject(new Error(JSON.stringify({code:"reconnect_required", message:"refresh revoked"})));
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
    expect(useSessionStore.getState().warning).toBeNull();
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
  it("keeps an outage offline without opening Google and reports a retryable error", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const account = {user:{id:"account-1", name:"Owner", method:"google" as const}};
    invoke.mockRejectedValue(JSON.stringify({code:"service_unavailable", message:"Account service unavailable. Try again."}));
    useSessionStore.setState({auth:account});
    await useSessionStore.getState().reconnectAuth();
    expect(invoke.mock.calls.map(([, args]) => args?.operation)).toEqual(["account.refresh"]);
    expect(useSessionStore.getState().auth?.connection).toBe("offline");
    expect(useSessionStore.getState().authReconnectError).toContain("Try again");
    expect(useSessionStore.getState().authReconnecting).toBe(false);
  });

  it("coalesces repeated reconnect clicks and preserves live work through interactive sign-in", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const account = {user:{id:"account-1", name:"Owner", method:"google" as const}};
    let finish: (value: unknown) => void = () => {};
    invoke.mockImplementation((_: string, args: {operation:string}) => {
      if (args.operation === "account.refresh") return Promise.reject(JSON.stringify({code:"reconnect_required", message:"Refresh rejected"}));
      if (args.operation === "account.sign_in") return new Promise((resolve) => { finish=resolve; });
      throw new Error(args.operation);
    });
    const syncCloudIndex = vi.fn(async () => {});
    const session = {id:"live-1"} as Session;
    useSessionStore.setState({auth:account, session, runningIds:["live-1"], syncCloudIndex});
    const first=useSessionStore.getState().reconnectAuth();
    const second=useSessionStore.getState().reconnectAuth();
    expect(second).toBe(first);
    expect(useSessionStore.getState().authReconnecting).toBe(true);
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
    finish({...account, connection:"ready"});
    await first;
    expect(useSessionStore.getState().session).toBe(session);
    expect(useSessionStore.getState().runningIds).toEqual(["live-1"]);
    expect(syncCloudIndex).toHaveBeenCalledOnce();
    expect(useSessionStore.getState().authReconnecting).toBe(false);
  });

  it("does not switch accounts or discard local sessions after a wrong-account browser response", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const account = {user:{id:"account-1", name:"Owner", method:"google" as const}};
    invoke.mockRejectedValueOnce(JSON.stringify({code:"reconnect_required", message:"Refresh rejected"}))
      .mockResolvedValueOnce({user:{id:"account-2", name:"Other", method:"google"}});
    const session={id:"live-1"} as Session;
    useSessionStore.setState({auth:account, session});
    await useSessionStore.getState().reconnectAuth();
    expect(useSessionStore.getState().auth?.user.id).toBe("account-1");
    expect(useSessionStore.getState().session).toBe(session);
    expect(useSessionStore.getState().authReconnectError).toContain("same account");
  });

  it("honors cancellation during renewal before a browser sign-in can start", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const account = {user:{id:"account-1", name:"Owner", method:"google" as const}};
    let reject: (reason: unknown) => void = () => {};
    invoke.mockImplementation((_: string, args: {operation:string}) => {
      if (args.operation === "account.refresh") return new Promise((_, fail) => { reject = fail; });
      if (args.operation === "account.cancel_sign_in") return Promise.resolve();
      throw new Error("A cancelled reconnect must not open Google");
    });
    useSessionStore.setState({auth:account});
    const reconnect=useSessionStore.getState().reconnectAuth();
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledOnce());
    await useSessionStore.getState().cancelAuthReconnect();
    reject(JSON.stringify({code:"reconnect_required", message:"Refresh rejected"}));
    await reconnect;
    expect(invoke.mock.calls.map(([, args]) => args?.operation)).toEqual(["account.refresh","account.cancel_sign_in"]);
    expect(useSessionStore.getState().authReconnectError).toContain("cancelled");
    expect(useSessionStore.getState().authReconnecting).toBe(false);
  });

});
