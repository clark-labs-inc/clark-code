import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot, type Session, type Snapshot } from "../core-bridge/types";
import { useSessionStore } from "./sessionStore";

const session: Session = {
  id: "chat-edit",
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

function bridgeStub(): CoreBridge {
  return {
    listProviders: async () => [{ id: "local", label: "Agent Desktop", capabilities: session.capabilities }],
    openSession: vi.fn(async () => session),
    closeSession: vi.fn(async () => {}),
    prompt: vi.fn(async () => ({ runId: "run-stub" })),
    cancel: vi.fn(async () => {}),
    respond: vi.fn(async () => {}),
    subscribe: () => () => {},
  };
}

beforeEach(() => {
  useSessionStore.getState().endSession({ force: true });
  useSessionStore.setState({
    bridge: null,
    activeProvider: "local",
    session: null,
    snapshot: emptySnapshot(),
    auth: null,
    connecting: false,
    opening: null,
    attachments: [],
    queued: [],
    conversations: [],
    localSettings: {
      cwd: "/tmp/project",
      model: "local-model",
      reasoningEffort: "high",
    },
    chatModels: {},
    activeRemote: null,
    activeRemoteHost: null,
    activeProjectRoot: null,
    approvalPolicy: "auto",
  });
});

describe("edit and resend", () => {
  it("replaces the selected turn and resumes the model from only the retained prefix", async () => {
    const bridge = bridgeStub();
    useSessionStore.setState({ bridge });
    await useSessionStore.getState().startSession();

    const snapshot: Snapshot = {
      session: session.id,
      runs: { r1: { id: "r1", status: "done" }, r2: { id: "r2", status: "failed" } },
      timeline: [
        { item: "message", run: "user", role: "user", blocks: [{ type: "text", text: "first" }] },
        { item: "message", run: "r1", role: "agent", blocks: [{ type: "text", text: "first reply" }] },
        { item: "message", run: "user", role: "user", blocks: [{ type: "text", text: "old second" }] },
        { item: "message", run: "r2", role: "agent", blocks: [{ type: "text", text: "failed reply" }] },
      ],
      tool_calls: {},
      artifacts: [],
      provider_incidents: {},
    };
    useSessionStore.setState({ snapshot });

    await useSessionStore.getState().resendFrom(2, "edited second");

    const openSession = vi.mocked(bridge.openSession);
    expect(openSession).toHaveBeenCalledTimes(2);
    expect(openSession.mock.calls[1]?.[2]).toMatchObject({
      kind: "new",
      bindId: session.id,
      options: {
      mode: "auto",
      collaboration_mode: "default",
      },
    });
    expect(openSession.mock.calls[1]?.[1].extra).toMatchObject({
      model: "local-model",
    });
    const request = openSession.mock.calls[1]?.[2];
    const resume = request?.kind === "new" ? request.options.resume : undefined;
    expect(JSON.stringify(resume)).toContain("first reply");
    expect(JSON.stringify(resume)).not.toContain("old second");
    expect(JSON.stringify(resume)).not.toContain("failed reply");
    expect(bridge.prompt).toHaveBeenCalledWith(
      session.id,
      [{ type: "text", text: "edited second" }],
      [],
    );
    expect(useSessionStore.getState().snapshot.timeline).toEqual(snapshot.timeline.slice(0, 2));
  });
});
