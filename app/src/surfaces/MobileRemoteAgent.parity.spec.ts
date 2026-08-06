import { beforeEach, describe, expect, it, vi } from "vitest";

import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot, type Session, type Snapshot } from "../core-bridge/types";
import type { CodeRemoteCommand } from "../lib/mobileRemote";
import { useSessionStore } from "../store/sessionStore";
import {
  liveSessions,
  snapshotCache,
} from "../store/sessionStore.runtime";
import {
  cancelRun,
  compactConversation,
  editAndResend,
  steerRun,
} from "./MobileRemoteAgent";

const session: Session = {
  id: "mobile-parity-session",
  provider: "local",
  capabilities: {
    streaming: true,
    permissions: true,
    fs: true,
    terminal: true,
    load_session: false,
    modes: [],
    collaboration_modes: ["default", "plan"],
  },
  collaboration_mode: "default",
  environment: {
    checkout_root: "/tmp/project",
    workspace_roots: ["/tmp/project"],
    remote: false,
  },
};
const originalOpenConversation = useSessionStore.getState().openConversation;

function bridgeStub(): CoreBridge {
  return {
    listProviders: async () => [{ id: "local", label: "Clark Code", capabilities: session.capabilities }],
    openSession: vi.fn(async () => session),
    closeSession: vi.fn(async () => {}),
    prompt: vi.fn(async () => ({ runId: "rerun-from-phone" })),
    cancel: vi.fn(async () => {}),
    compact: vi.fn(async () => {}),
    respond: vi.fn(async () => {}),
    steer: vi.fn(async () => {}),
    subscribe: () => () => {},
  };
}

function command(
  commandType: CodeRemoteCommand["command_type"],
  request: Record<string, unknown>,
): CodeRemoteCommand {
  return {
    command_id: `command-${commandType}`,
    host_id: "host-1",
    project_id: "local:%2Ftmp%2Fproject",
    desktop_id: session.id,
    command_type: commandType,
    request,
    status: "delivered",
    created_at: "2026-07-26T00:00:00Z",
    updated_at: "2026-07-26T00:00:01Z",
  };
}

async function openWithSnapshot(snapshot: Snapshot): Promise<CoreBridge> {
  const bridge = bridgeStub();
  useSessionStore.setState({ bridge });
  await useSessionStore.getState().startSession();
  const entry = liveSessions.get(session.id);
  if (!entry) throw new Error("test session did not become live");
  entry.live = snapshot;
  useSessionStore.setState({ snapshot });
  return bridge;
}

beforeEach(() => {
  useSessionStore.getState().endSession({ force: true });
  liveSessions.clear();
  snapshotCache.clear();
  useSessionStore.setState({
    bridge: null,
    activeProvider: "local",
    session: null,
    snapshot: emptySnapshot(),
    auth: null,
    connecting: false,
    opening: null,
    openConversation: originalOpenConversation,
    attachments: [],
    queued: [],
    conversations: [],
    localSettings: {
      cwd: "/tmp/project",
      model: "clark-code:free",
      reasoningEffort: "max",
    },
    chatModels: {},
    activeRemote: null,
    activeRemoteHost: null,
    activeProjectRoot: null,
    approvalPolicy: "auto",
  });
});

describe("Clark Mobile parity commands", () => {
  it("classifies a recovered stale run without reopening its large transcript", async () => {
    const restored = {
      ...emptySnapshot(),
      session: session.id,
      runs: { "run-interrupted": { id: "run-interrupted", status: "failed" } },
    } as Snapshot;
    snapshotCache.set(session.id, restored);
    const openConversation = vi.fn(async () => {});
    useSessionStore.setState({ openConversation });

    await expect(cancelRun(command("cancel_run", {
      run_id: "run-interrupted",
    }))).rejects.toMatchObject({
      code: "stale_run",
      retryable: false,
    });

    expect(openConversation).not.toHaveBeenCalled();
  });

  it("injects an explicit phone steer into the exact active run", async () => {
    const snapshot = {
      ...emptySnapshot(),
      session: session.id,
      runs: { "run-active": { id: "run-active", status: "running" } },
    } as Snapshot;
    const bridge = await openWithSnapshot(snapshot);

    await steerRun(command("steer_run", {
      run_id: "run-active",
      text: "focus on the failing test",
    }));

    expect(bridge.steer).toHaveBeenCalledWith(session.id, [
      { type: "text", text: "focus on the failing test" },
    ]);
  });

  it("compacts the exact idle live conversation", async () => {
    const bridge = await openWithSnapshot({
      ...emptySnapshot(),
      session: session.id,
    });

    await compactConversation(command("compact_conversation", {}));

    expect(bridge.compact).toHaveBeenCalledWith(session.id);
  });

  it("rejects snapshot drift and reruns from the exact edited phone turn", async () => {
    const snapshot: Snapshot = {
      session: session.id,
      runs: {
        r1: { id: "r1", status: "done" },
        r2: { id: "r2", status: "failed" },
      },
      timeline: [
        { item: "message", run: "user-1", role: "user", blocks: [{ type: "text", text: "first" }] },
        { item: "message", run: "r1", role: "agent", blocks: [{ type: "text", text: "first reply" }] },
        { item: "message", run: "user-2", role: "user", blocks: [{ type: "text", text: "old second" }] },
        { item: "message", run: "r2", role: "agent", blocks: [{ type: "text", text: "failed reply" }] },
      ],
      tool_calls: {},
      artifacts: [],
      provider_incidents: {},
    };
    const bridge = await openWithSnapshot(snapshot);
    const edit = command("edit_and_resend", {
      text: "edited second",
      payload: {
        timeline_index: 2,
        expected_text: "old second",
      },
    });

    const stale = {
      ...edit,
      request: {
        ...edit.request,
        payload: { timeline_index: 2, expected_text: "different text" },
      },
    };
    await expect(editAndResend(stale)).rejects.toMatchObject({ code: "stale_edit" });

    await expect(editAndResend(edit)).resolves.toEqual({ runId: "rerun-from-phone" });
    expect(bridge.prompt).toHaveBeenCalledWith(
      session.id,
      [{ type: "text", text: "edited second" }],
      [],
    );
    expect(useSessionStore.getState().snapshot.timeline).toEqual(snapshot.timeline.slice(0, 2));
  });
});
