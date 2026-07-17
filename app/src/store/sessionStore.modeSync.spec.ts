import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionStore } from "./sessionStore";
import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot, type Session } from "../core-bridge/types";

// Every permission-mode change must reach the engine: the local agent's
// plan-mode gate (read-only until the plan is approved) lives server-side, so
// a composer-pill pick that only updates client state silently degrades plan
// mode to "ask for everything".

const session = { id: "sess-1", provider: "local" } as unknown as Session;

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
    subscribe: () => () => {},
    ...overrides,
  } as CoreBridge;
}

beforeEach(() => {
  useSessionStore.setState({
    bridge: null,
    session: null,
    snapshot: emptySnapshot(),
    permissionMode: "auto",
    activeProvider: "local",
    auth: null,
    connecting: false,
    opening: null,
    queued: [],
  });
});

describe("permission-mode ↔ engine sync", () => {
  it("setPermissionMode pushes the mode to the engine", () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, session });

    useSessionStore.getState().setPermissionMode("plan");

    expect(useSessionStore.getState().permissionMode).toBe("plan");
    expect(bridge.setMode).toHaveBeenCalledWith("sess-1", "plan");
  });

  it("cyclePermissionMode syncs the engine exactly once per cycle", () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, session, permissionMode: "auto" });

    useSessionStore.getState().cyclePermissionMode();

    // MODE_CYCLE: ask → auto → full → plan.
    expect(useSessionStore.getState().permissionMode).toBe("full");
    expect(bridge.setMode).toHaveBeenCalledTimes(1);
    expect(bridge.setMode).toHaveBeenCalledWith("sess-1", "full");
  });

  it("setPermissionMode without a live session only updates client state", () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, session: null });

    useSessionStore.getState().setPermissionMode("plan");

    expect(useSessionStore.getState().permissionMode).toBe("plan");
    expect(bridge.setMode).not.toHaveBeenCalled();
  });

  it("startSession passes the composer mode to newSession", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, permissionMode: "plan" });

    await useSessionStore.getState().startSession();

    expect(bridge.newSession).toHaveBeenCalledWith(
      "local",
      expect.objectContaining({ mode: "plan" }),
    );
  });

  it("startSession does NOT send the permission mode to the cloud provider", async () => {
    // `SessionOptions.mode` is provider-defined: for the Clark cloud provider
    // it selects the TIER (clark/clark_max), so the client permission mode
    // must never ride along — it would corrupt the tier.
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, permissionMode: "plan", activeProvider: "clark" });

    await useSessionStore.getState().startSession();

    const optionsArg = vi.mocked(bridge.newSession).mock.calls[0][1];
    expect(optionsArg.mode).toBeUndefined();
  });

  it("openConversation passes the composer mode to newSession", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({
      bridge,
      permissionMode: "plan",
      localSettings: {
        ...useSessionStore.getState().localSettings,
        cwd: "/tmp/project",
      },
    });

    await useSessionStore.getState().openConversation("conv-reopen");

    expect(bridge.newSession).toHaveBeenCalledWith(
      "local",
      expect.objectContaining({ mode: "plan" }),
      "conv-reopen",
    );
  });
});
