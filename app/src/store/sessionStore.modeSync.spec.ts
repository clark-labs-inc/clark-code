import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionStore } from "./sessionStore";
import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot, type Session } from "../core-bridge/types";
import { liveSessions, newLiveEntry, effectiveApprovalPolicy, effectiveCollaborationMode } from "./sessionStore.runtime";

const session = {
  id: "sess-1",
  provider: "local",
  collaboration_mode: "default",
} as unknown as Session;

const sessionB = {
  id: "sess-2",
  provider: "local",
  collaboration_mode: "default",
} as unknown as Session;

function stubBridge(overrides: Partial<CoreBridge> = {}): CoreBridge {
  return {
    listProviders: async () => [],
    openSession: vi.fn(async () => session),
    prompt: async () => {},
    cancel: vi.fn(async () => {}),
    respond: vi.fn(async () => {}),
    setMode: vi.fn(async () => {}),
    setCollaborationMode: vi.fn(async () => {}),
    subscribe: () => () => {},
    ...overrides,
  } as CoreBridge;
}

beforeEach(() => {
  liveSessions.clear();
  useSessionStore.setState({
    bridge: null,
    session: null,
    snapshot: emptySnapshot(),
    approvalPolicy: "auto",
    approvalPolicies: {},
    collaborationMode: "default",
    collaborationModes: {},
    activeProvider: "local",
    auth: null,
    connecting: false,
    opening: null,
    queued: [],
  });
});

describe("approval and collaboration mode", () => {
  it("changes the focused chat's approval level and syncs only its executor mode", () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, session });
    useSessionStore.getState().setApprovalPolicy("full");
    // The account-wide default is untouched — only this chat's override moved.
    expect(useSessionStore.getState().approvalPolicy).toBe("auto");
    expect(useSessionStore.getState().approvalPolicies["sess-1"]).toBe("full");
    expect(useSessionStore.getState().session?.mode).toBe("full");
    expect(bridge.setMode).toHaveBeenCalledWith("sess-1", "full");
    expect(bridge.setMode).toHaveBeenCalledTimes(1);
    expect(bridge.setCollaborationMode).not.toHaveBeenCalled();
  });

  it("changes the global default when no chat is open (start screen)", () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, session: null });
    useSessionStore.getState().setApprovalPolicy("full");
    expect(useSessionStore.getState().approvalPolicy).toBe("full");
    expect(useSessionStore.getState().approvalPolicies).toEqual({});
    expect(bridge.setMode).not.toHaveBeenCalled();
  });

  it("setDefaultApprovalPolicy edits only the account-wide default", () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, session });
    useSessionStore.getState().setDefaultApprovalPolicy("ask");
    expect(useSessionStore.getState().approvalPolicy).toBe("ask");
    expect(useSessionStore.getState().approvalPolicies).toEqual({});
    // The open chat's own level and host mode are left alone.
    expect(useSessionStore.getState().session?.mode).toBeUndefined();
    expect(bridge.setMode).not.toHaveBeenCalled();
  });

  it("syncs collaboration mode independently per chat", () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, session });
    useSessionStore.getState().setCollaborationMode("plan");
    // Only THIS chat's override moved; the account-wide default is untouched.
    expect(useSessionStore.getState().collaborationModes["sess-1"]).toBe("plan");
    expect(useSessionStore.getState().collaborationMode).toBe("default");
    expect(bridge.setCollaborationMode).toHaveBeenCalledWith("sess-1", "plan");
  });

  it("cycles the focused chat's effective level (override, else default)", () => {
    const bridge = stubBridge();
    // Chat has no override, so effective = global default "full" → cycles to "ask".
    useSessionStore.setState({ bridge, session, approvalPolicy: "full" });
    useSessionStore.getState().cycleApprovalPolicy();
    // The override is pinned on this chat; the global default is untouched.
    expect(useSessionStore.getState().approvalPolicies["sess-1"]).toBe("ask");
    expect(useSessionStore.getState().approvalPolicy).toBe("full");
    expect(bridge.setMode).toHaveBeenCalledWith("sess-1", "ask");
  });

  it("does not cycle an invisible local approval policy for cloud sessions", () => {
    const bridge = stubBridge();
    const cloudSession = { id: "conv-9", provider: "product-cloud" } as unknown as Session;
    useSessionStore.setState({ bridge, session: cloudSession, approvalPolicy: "auto" });
    useSessionStore.getState().cycleApprovalPolicy();
    expect(useSessionStore.getState().approvalPolicy).toBe("auto");
    expect(useSessionStore.getState().approvalPolicies).toEqual({});
    expect(bridge.setMode).not.toHaveBeenCalled();
  });
});

describe("per-chat approval isolation", () => {
  // Shift+Tab cycling approval in one conversation must not change what any
  // other conversation runs. Before the fix a single global setting was the
  // only level, and setApprovalPolicy rewrote it for every live session —
  // exactly the "cycles across all chat, not one chat in focus" complaint.

  it("changing approval in one chat does not affect another's effective level", () => {
    const bridge = stubBridge();
    // Two live local chats in the background pool.
    liveSessions.set(session.id, newLiveEntry(session, { historyPrefix: null, remote: null, remoteHost: null, projectRoot: "/a" }));
    liveSessions.set(sessionB.id, newLiveEntry(sessionB, { historyPrefix: null, remote: null, remoteHost: null, projectRoot: "/b" }));
    useSessionStore.setState({ bridge, session });

    // Focus chat A and cycle it from the default "auto" to "full".
    useSessionStore.getState().cycleApprovalPolicy();
    expect(useSessionStore.getState().approvalPolicies["sess-1"]).toBe("full");

    // Chat B still runs under the account default "auto" — its own override is
    // untouched, and the host was never told to change it.
    const state = useSessionStore.getState();
    expect(state.approvalPolicies["sess-2"]).toBeUndefined();
    expect(effectiveApprovalPolicy(state.approvalPolicy, state.approvalPolicies, "sess-2")).toBe("auto");

    // setMode was called exactly once — for chat A only, never chat B.
    expect(bridge.setMode).toHaveBeenCalledTimes(1);
    expect(bridge.setMode).toHaveBeenCalledWith("sess-1", "full");
    expect(bridge.setMode).not.toHaveBeenCalledWith("sess-2", expect.anything());
  });
});

describe("per-chat collaboration isolation", () => {
  // Switching plan mode in one conversation must not leak into a sibling
  // session. Before the fix a single global setting was the only mode and
  // setCollaborationMode rewrote it for every chat — exactly the "planning
  // travels to another session" complaint.

  it("switching plan mode in one chat does not affect another's effective mode", () => {
    const bridge = stubBridge();
    // Two live local chats in the background pool.
    liveSessions.set(session.id, newLiveEntry(session, { historyPrefix: null, remote: null, remoteHost: null, projectRoot: "/a" }));
    liveSessions.set(sessionB.id, newLiveEntry(sessionB, { historyPrefix: null, remote: null, remoteHost: null, projectRoot: "/b" }));
    useSessionStore.setState({ bridge, session });

    // Focus chat A and put it into plan mode.
    useSessionStore.getState().setCollaborationMode("plan");
    expect(useSessionStore.getState().collaborationModes["sess-1"]).toBe("plan");

    // Chat B still runs under the account default — its own override is
    // untouched, and the host was never told to change B.
    const state = useSessionStore.getState();
    expect(state.collaborationModes["sess-2"]).toBeUndefined();
    expect(effectiveCollaborationMode(state.collaborationMode, state.collaborationModes, "sess-2")).toBe("default");

    // setCollaborationMode was called exactly once — for chat A only, never B.
    expect(bridge.setCollaborationMode).toHaveBeenCalledTimes(1);
    expect(bridge.setCollaborationMode).toHaveBeenCalledWith("sess-1", "plan");
    expect(bridge.setCollaborationMode).not.toHaveBeenCalledWith("sess-2", expect.anything());
  });

  it("sets the account-wide default when no chat is open (start screen)", () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, session: null });
    useSessionStore.getState().setCollaborationMode("plan");
    expect(useSessionStore.getState().collaborationMode).toBe("plan");
    expect(useSessionStore.getState().collaborationModes["sess-1"]).toBeUndefined();
    expect(bridge.setCollaborationMode).not.toHaveBeenCalled();
  });
});
