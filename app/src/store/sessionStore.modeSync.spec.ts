import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionStore } from "./sessionStore";
import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot, type Session } from "../core-bridge/types";

const session = {
  id: "sess-1",
  provider: "local",
  collaboration_mode: "default",
} as unknown as Session;

function stubBridge(overrides: Partial<CoreBridge> = {}): CoreBridge {
  return {
    listProviders: async () => [],
    connect: vi.fn(async () => {}),
    newSession: vi.fn(async () => session),
    loadSession: async () => session,
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
  useSessionStore.setState({
    bridge: null,
    session: null,
    snapshot: emptySnapshot(),
    approvalPolicy: "auto",
    collaborationMode: "default",
    activeProvider: "local",
    auth: null,
    connecting: false,
    opening: null,
    queued: [],
  });
});

describe("approval and collaboration mode", () => {
  it("changes approval policy and synchronizes the local executor mode", () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, session });
    useSessionStore.getState().setApprovalPolicy("full");
    expect(useSessionStore.getState().approvalPolicy).toBe("full");
    expect(useSessionStore.getState().session?.mode).toBe("full");
    expect(bridge.setMode).toHaveBeenCalledWith("sess-1", "full");
    expect(bridge.setCollaborationMode).not.toHaveBeenCalled();
  });

  it("syncs collaboration mode independently", () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, session });
    useSessionStore.getState().setCollaborationMode("plan");
    expect(useSessionStore.getState().collaborationMode).toBe("plan");
    expect(bridge.setCollaborationMode).toHaveBeenCalledWith("sess-1", "plan");
  });

  it("cycles only the three approval policies", () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, session, approvalPolicy: "full" });
    useSessionStore.getState().cycleApprovalPolicy();
    expect(useSessionStore.getState().approvalPolicy).toBe("ask");
    expect(bridge.setMode).toHaveBeenCalledWith("sess-1", "ask");
  });

  it("does not cycle an invisible local approval policy for cloud sessions", () => {
    const bridge = stubBridge();
    const cloudSession = { id: "conv-9", provider: "clark" } as unknown as Session;
    useSessionStore.setState({ bridge, session: cloudSession, approvalPolicy: "auto" });
    useSessionStore.getState().cycleApprovalPolicy();
    expect(useSessionStore.getState().approvalPolicy).toBe("auto");
    expect(bridge.setMode).not.toHaveBeenCalled();
  });
});
