import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionStore } from "./sessionStore";
import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot, type Session, type Snapshot } from "../core-bridge/types";
import { liveSessions, newLiveEntry } from "./sessionStore.runtime";

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
    openSession: vi.fn(async () => localSession),
    prompt: vi.fn(async () => ({ runId: "run-stub" })),
    cancel: vi.fn(async () => {}),
    respond: vi.fn(async () => {}),
    steer: vi.fn(async () => {}),
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
    attachments: [],
    queued: [],
    error: null,
    updateWaiting: false,
    updateApplying: false,
  });
});

describe("queued follow-ups and explicit steering", () => {
  it("suppresses rapid duplicate submits before the first prompt is visible", async () => {
    let release!: () => void;
    const promptGate = new Promise<void>((resolve) => { release = resolve; });
    const bridge = stubBridge({
      prompt: vi.fn(async () => {
        await promptGate;
        return { runId: "run-first" };
      }),
    });
    liveSessions.set(localSession.id, newLiveEntry(localSession, {
      historyPrefix: null,
      remote: null,
      remoteHost: null,
      projectRoot: "/tmp/project",
    }));
    useSessionStore.setState({
      bridge,
      session: localSession,
      snapshot: emptySnapshot(),
    });

    const first = useSessionStore.getState().send("hi");
    await Promise.resolve();
    expect(useSessionStore.getState().snapshot.timeline).toEqual([
      expect.objectContaining({
        item: "message",
        role: "user",
        blocks: [{ type: "text", text: "hi" }],
      }),
    ]);
    const second = useSessionStore.getState().send("hi");
    const third = useSessionStore.getState().send("hi");

    await expect(second).resolves.toEqual({ kind: "not_sent" });
    await expect(third).resolves.toEqual({ kind: "not_sent" });
    expect(bridge.prompt).toHaveBeenCalledTimes(1);
    expect(useSessionStore.getState().queued).toEqual([]);
    release();
    await first;
  });

  it("removes the optimistic bubble and restores the draft when dispatch fails", async () => {
    const bridge = stubBridge({
      prompt: vi.fn(async () => {
        throw new Error("transport unavailable");
      }),
    });
    liveSessions.set(localSession.id, newLiveEntry(localSession, {
      historyPrefix: null,
      remote: null,
      remoteHost: null,
      projectRoot: "/tmp/project",
    }));
    useSessionStore.setState({ bridge, session: localSession, snapshot: emptySnapshot() });

    const outcome = await useSessionStore.getState().send("keep my message");

    expect(outcome).toEqual({ kind: "not_sent" });
    expect(useSessionStore.getState().snapshot.timeline).toEqual([]);
    expect(useSessionStore.getState().composerPrefill).toEqual({ text: "keep my message" });
    expect(useSessionStore.getState().error).toContain("transport unavailable");
  });

  it("queues during an active local run instead of steering it", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({
      bridge,
      session: localSession,
      snapshot: busySnapshot("sess-1"),
    });

    const outcome = await useSessionStore.getState().send("actually, use pnpm");

    expect(outcome).toEqual({
      kind: "queued",
      queueId: expect.any(String),
    });
    expect(bridge.steer).not.toHaveBeenCalled();
    expect(bridge.prompt).not.toHaveBeenCalled();
    expect(useSessionStore.getState().queued.map((q) => q.text)).toEqual([
      "actually, use pnpm",
    ]);
  });

  it("treats an exact stop message during an active run as cancellation", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({
      bridge,
      session: localSession,
      snapshot: busySnapshot("sess-1"),
    });

    const outcome = await useSessionStore.getState().send("stop");

    expect(outcome).toEqual({ kind: "cancelled" });
    expect(bridge.cancel).toHaveBeenCalledWith("sess-1", "run-1");
    expect(bridge.steer).not.toHaveBeenCalled();
    expect(bridge.prompt).not.toHaveBeenCalled();
    expect(useSessionStore.getState().queued).toEqual([]);
  });

  it("surfaces cancellation failures and keeps the stop draft", async () => {
    const bridge = stubBridge({
      cancel: vi.fn(async () => {
        throw new Error("cancel transport unavailable");
      }),
    });
    useSessionStore.setState({
      bridge,
      session: localSession,
      snapshot: busySnapshot("sess-1"),
    });

    const outcome = await useSessionStore.getState().send("stop");

    expect(outcome).toEqual({ kind: "not_sent" });
    expect(useSessionStore.getState().error).toContain("cancel transport unavailable");
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

  it("cancels instead of steering an exact stop already in the queue", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({
      bridge,
      session: localSession,
      snapshot: busySnapshot("sess-1"),
      queued: [{ id: "stop-1", text: "stop", uploads: [], skills: [] }],
    });

    await useSessionStore.getState().steerQueued("stop-1");

    expect(bridge.cancel).toHaveBeenCalledWith("sess-1", "run-1");
    expect(bridge.steer).not.toHaveBeenCalled();
    expect(useSessionStore.getState().queued).toEqual([]);
  });

  it("keeps a queued stop and surfaces the error when cancellation fails", async () => {
    const bridge = stubBridge({
      cancel: vi.fn(async () => {
        throw new Error("cancel bridge failed");
      }),
    });
    useSessionStore.setState({
      bridge,
      session: localSession,
      snapshot: busySnapshot("sess-1"),
      queued: [{ id: "stop-1", text: "stop", uploads: [], skills: [] }],
    });

    await useSessionStore.getState().steerQueued("stop-1");

    expect(useSessionStore.getState().queued).toHaveLength(1);
    expect(useSessionStore.getState().error).toContain("cancel bridge failed");
  });

  it("cancelActive surfaces bridge failures instead of failing silently", async () => {
    const bridge = stubBridge({
      cancel: vi.fn(async () => {
        throw new Error("native cancel failed");
      }),
    });
    useSessionStore.setState({
      bridge,
      session: localSession,
      snapshot: busySnapshot("sess-1"),
    });

    await useSessionStore.getState().cancelActive();

    expect(bridge.cancel).toHaveBeenCalledWith("sess-1", "run-1");
    expect(useSessionStore.getState().error).toContain("native cancel failed");
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
    expect(receipt).toEqual({
      kind: "started",
      receipt: { runId: "run-stub" },
    });
  });

  it("reports update-gated messages as not sent without consuming the draft upstream", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({
      bridge,
      session: localSession,
      snapshot: emptySnapshot(),
      updateWaiting: true,
    });

    const outcome = await useSessionStore.getState().send("send after restart");

    expect(outcome).toEqual({ kind: "not_sent" });
    expect(bridge.prompt).not.toHaveBeenCalled();
  });
});
