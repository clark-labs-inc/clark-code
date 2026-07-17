import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionStore } from "./sessionStore";
import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot, type Session, type Snapshot } from "../core-bridge/types";

// A message sent while a LOCAL run is active steers the live run (injected
// between tool batches, Codex-style) instead of waiting in the queue until
// the run ends. Cloud runs and failures fall back to the queue.

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
    prompt: vi.fn(async () => {}),
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

describe("mid-run steering", () => {
  it("steers the active local run instead of queueing", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({
      bridge,
      session: localSession,
      snapshot: busySnapshot("sess-1"),
    });

    await useSessionStore.getState().send("actually, use pnpm");

    expect(bridge.steer).toHaveBeenCalledWith("sess-1", [
      { type: "text", text: "actually, use pnpm" },
    ]);
    expect(bridge.prompt).not.toHaveBeenCalled();
    expect(useSessionStore.getState().queued).toEqual([]);
  });

  it("falls back to the queue when the run just ended", async () => {
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

    expect(useSessionStore.getState().queued.map((q) => q.text)).toEqual(["follow-up"]);
    expect(bridge.prompt).not.toHaveBeenCalled();
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

    await useSessionStore.getState().send("fresh turn");

    expect(bridge.prompt).toHaveBeenCalled();
    expect(bridge.steer).not.toHaveBeenCalled();
  });
});
