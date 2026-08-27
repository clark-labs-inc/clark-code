import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionStore } from "./sessionStore";
import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot, type Session, type Snapshot } from "../core-bridge/types";
import {
  liveSessions,
  newLiveEntry,
  restoreQueuedAfterDispatchFailure,
} from "./sessionStore.runtime";

// Messages sent during any active run queue by default. Explicit steering
// cancels the current run and keeps the message queued for the normal drain.

const localSession = { id: "sess-1", provider: "local" } as unknown as Session;
const cloudSession = { id: "conv-1", provider: "local" } as unknown as Session;

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

  it("does not queue a rapid duplicate after the first submit consumes an attachment", async () => {
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
      attachments: [{
        id: "attachment-1",
        filename: "benchmark.png",
        content_type: "image/png",
        data_base64: "cG5n",
        size: 3,
      }],
    });

    const first = useSessionStore.getState().send("explain this benchmark");
    await Promise.resolve();
    expect(useSessionStore.getState().attachments).toEqual([]);

    const duplicate = useSessionStore.getState().send("explain this benchmark");

    await expect(duplicate).resolves.toEqual({ kind: "not_sent" });
    expect(bridge.prompt).toHaveBeenCalledTimes(1);
    expect(bridge.prompt).toHaveBeenCalledWith(
      localSession.id,
      [{ type: "text", text: "explain this benchmark" }],
      [{
        filename: "benchmark.png",
        content_type: "image/png",
        data_base64: "cG5n",
      }],
    );
    expect(useSessionStore.getState().queued).toEqual([]);
    expect(useSessionStore.getState().snapshot.timeline).toHaveLength(1);
    release();
    await first;
  });

  it("suppresses the same message for the full admission window", async () => {
    let release!: () => void;
    const promptGate = new Promise<void>((resolve) => { release = resolve; });
    const bridge = stubBridge({
      prompt: vi.fn(async () => {
        await promptGate;
        return { runId: "run-first" };
      }),
    });
    const entry = newLiveEntry(localSession, {
      historyPrefix: null,
      remote: null,
      remoteHost: null,
      projectRoot: "/tmp/project",
    });
    liveSessions.set(localSession.id, entry);
    useSessionStore.setState({
      bridge,
      session: localSession,
      snapshot: emptySnapshot(),
    });

    const first = useSessionStore.getState().send("explain the attachment");
    await Promise.resolve();
    entry.lastSubmittedAt = Date.now() - 10_000;

    await expect(
      useSessionStore.getState().send("explain the attachment"),
    ).resolves.toEqual({ kind: "not_sent" });
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

  it("restores consumed attachments when dispatch fails", async () => {
    const attachment = {
      id: "attachment-retry",
      filename: "routing.png",
      content_type: "image/png",
      data_base64: "cG5n",
      size: 3,
    };
    const bridge = stubBridge({
      prompt: vi.fn(async () => {
        throw new Error("upload transport unavailable");
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
      attachments: [attachment],
    });

    const outcome = await useSessionStore.getState().send("explain this");

    expect(outcome).toEqual({ kind: "not_sent" });
    expect(useSessionStore.getState().attachments).toEqual([attachment]);
    expect(useSessionStore.getState().composerPrefill).toEqual({ text: "explain this" });
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

  it("stops current work and keeps the steering message queued for the next run", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({
      bridge,
      session: localSession,
      snapshot: busySnapshot("sess-1"),
    });

    await useSessionStore.getState().send("follow-up");
    const [queued] = useSessionStore.getState().queued;
    await useSessionStore.getState().steerQueued(queued.id);

    expect(bridge.cancel).toHaveBeenCalledWith("sess-1", "run-1");
    expect(bridge.steer).not.toHaveBeenCalled();
    expect(useSessionStore.getState().queued.map((q) => q.text)).toEqual(["follow-up"]);
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

  it("keeps the message queued when steering cannot stop the active run", async () => {
    const bridge = stubBridge({
      cancel: vi.fn(async () => {
        throw new Error("no active run to stop");
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

  it("restores a steered follow-up if its next-run prompt is rejected", () => {
    const entry = newLiveEntry(localSession, {
      historyPrefix: null,
      remote: null,
      remoteHost: null,
      projectRoot: "/tmp/project",
    });
    const steered = { id: "steer-1", text: "use vLLM", uploads: [], skills: [] };
    entry.queued = [{ id: "later-1", text: "then verify it", uploads: [], skills: [] }];

    restoreQueuedAfterDispatchFailure(entry, steered);
    restoreQueuedAfterDispatchFailure(entry, steered);

    expect(entry.queued.map((message) => message.text)).toEqual([
      "use vLLM",
      "then verify it",
    ]);
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
