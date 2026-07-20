import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot, type Session } from "../core-bridge/types";
import type { PendingAttachment } from "../lib/attachments";
import { useSessionStore } from "./sessionStore";

const session: Session = {
  id: "attachment-session",
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
    listProviders: async () => [],
    connect: vi.fn(async () => {}),
    newSession: vi.fn(async () => session),
    loadSession: vi.fn(async () => session),
    prompt: vi.fn(async () => {}),
    cancel: vi.fn(async () => {}),
    respond: vi.fn(async () => {}),
    subscribe: () => () => {},
  } as CoreBridge;
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
      model: "clark-code:grok45",
      reasoningEffort: "high",
      apiKey: "test-key",
    },
  });
});

describe("first-turn attachments", () => {
  it("keeps staged files while creating the session and sends them with the first prompt", async () => {
    const bridge = bridgeStub();
    const attachment: PendingAttachment = {
      id: "attachment-1",
      filename: "brief.txt",
      content_type: "text/plain",
      data_base64: "YnJpZWY=",
      size: 5,
    };
    useSessionStore.setState({ bridge, attachments: [attachment] });

    await useSessionStore.getState().startSession();

    expect(useSessionStore.getState().attachments).toEqual([attachment]);

    await useSessionStore.getState().send("");

    expect(bridge.prompt).toHaveBeenCalledWith(
      session.id,
      [{ type: "text", text: "" }],
      [
        {
          filename: "brief.txt",
          content_type: "text/plain",
          data_base64: "YnJpZWY=",
        },
      ],
    );
    expect(useSessionStore.getState().attachments).toEqual([]);
  });
});
