import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionStore } from "./sessionStore";
import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot, type Session, type Snapshot } from "../core-bridge/types";

// Messages sent during any active run queue by default. A queued, text-only
// local message can still steer the live run when the user explicitly asks.

const localSession = { id: "sess-1", provider: "local" } as unknown as Session;
const cloudSession = { id: "conv-1", provider: "clark" } as unknown as Session;

function busySnapshot(sessionId: string): Snapshot {
  return {
    ...emptySnapshot(),
    session: sessionId,
    runs: { "run-1": { id: "run-1", status: "running" } },
  } as Snapshot;
}

function stubBridge(overrides: Partial<CoreBridge> = {}): CoreBridge {
  return {
    listProviders: async () => [],
    connect: vi.fn(async () => {}),
    newSession: vi.fn(async () => localSession),
    loadSession: async () => localSession,
    prompt: vi.fn(async () => ({ runId: "run-stub" })),
    cancel: vi.fn(async () => {}),
    respond: vi.fn(async () => {}),
    steer: vi.fn(async () => {}),
    subscribe: () => () => {},
    ...overrides,
  } as CoreBridge;
}

beforeEach(() => {
  useSessionStore.setState({
    bridge: null,
    session: null,
    snapshot: emptySnapshot(),
    attachments: [],
    queued: [],
    error: null,
  });
});

describe("queued follow-ups and explicit steering", () => {
  it("queues during an active local run instead of steering it", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({
      bridge,
      session: localSession,
      snapshot: busySnapshot("sess-1"),
    });

    await useSessionStore.getState().send("actually, use pnpm");

    expect(bridge.steer).not.toHaveBeenCalled();
    expect(bridge.prompt).not.toHaveBeenCalled();
    expect(useSessionStore.getState().queued.map((q) => q.text)).toEqual([
      "actually, use pnpm",
    ]);
  });

  it("steers only when explicitly requested, then removes the queued message", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({
      bridge,
      session: localSession,
      snapshot: busySnapshot("sess-1"),
    });

    await useSessionStore.getState().send("follow-up");
    const [queued] = useSessionStore.getState().queued;
    await useSessionStore.getState().steerQueued(queued.id);

    expect(bridge.steer).toHaveBeenCalledWith("sess-1", [
      { type: "text", text: "follow-up" },
    ]);
    expect(useSessionStore.getState().queued).toEqual([]);
    expect(bridge.prompt).not.toHaveBeenCalled();
  });

  it("keeps the message queued when explicit steering loses the active-run race", async () => {
    const bridge = stubBridge({
      steer: vi.fn(async () => {
        throw new Error("no active run to steer");
      }),
    });
    useSessionStore.setState({
      bridge,
      session: localSession,
      snapshot: busySnapshot("sess-1"),
    });

    await useSessionStore.getState().send("follow-up");
    const [queued] = useSessionStore.getState().queued;
    await useSessionStore.getState().steerQueued(queued.id);

    expect(useSessionStore.getState().queued.map((q) => q.text)).toEqual(["follow-up"]);
  });

  it("cloud sessions keep the queue behavior", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({
      bridge,
      session: cloudSession,
      snapshot: busySnapshot("conv-1"),
    });

    await useSessionStore.getState().send("cloud follow-up");

    expect(bridge.steer).not.toHaveBeenCalled();
    expect(useSessionStore.getState().queued.map((q) => q.text)).toEqual(["cloud follow-up"]);
  });

  it("idle sessions still prompt directly", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({
      bridge,
      session: localSession,
      snapshot: emptySnapshot(),
    });

    const receipt = await useSessionStore.getState().send("fresh turn");

    expect(bridge.prompt).toHaveBeenCalled();
    expect(bridge.steer).not.toHaveBeenCalled();
    expect(receipt).toEqual({ runId: "run-stub" });
  });
});
