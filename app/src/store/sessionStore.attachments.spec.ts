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
    openSession: vi.fn(async () => session),
    prompt: vi.fn(async () => ({ runId: "run-stub" })),
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
      model: "local-model",
      reasoningEffort: "max",
    },
  });
});

describe("first-turn attachments", () => {
  it("drops a file whose encoding finishes after the composer changes", async () => {
    let finishEncoding!: (buffer: ArrayBuffer) => void;
    const encoding = new Promise<ArrayBuffer>((resolve) => {
      finishEncoding = resolve;
    });
    const file = new File(["draft"], "draft.txt", { type: "text/plain" });
    Object.defineProperty(file, "arrayBuffer", {
      configurable: true,
      value: () => encoding,
    });

    const adding = useSessionStore.getState().addFiles([file]);
    useSessionStore.getState().endSession();
    finishEncoding(new TextEncoder().encode("draft").buffer);
    await adding;

    expect(useSessionStore.getState().attachments).toEqual([]);
  });

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

  it("rejects unsupported media before encoding or sending it", async () => {
    useSessionStore.setState({
      activeProvider: "specialist",
      providers: [{
        id: "specialist",
        label: "Specialist",
        capabilities: {
          streaming: true,
          permissions: false,
          fs: false,
          terminal: false,
          load_session: true,
          attachment_kinds: [],
          modes: [],
          collaboration_modes: ["default"],
        },
      }],
    });

    await useSessionStore.getState().addFiles([
      new File(["audio"], "voice.wav", { type: "audio/wav" }),
    ]);

    expect(useSessionStore.getState().attachments).toEqual([]);
    expect(useSessionStore.getState().error).toContain("cannot ingest");
  });
});
