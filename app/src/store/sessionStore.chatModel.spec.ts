import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionStore } from "./sessionStore";
import type { CoreBridge, SessionOptions } from "../core-bridge/bridge";
import { emptySnapshot, type Session } from "../core-bridge/types";
import { effectiveModelSettings } from "../lib/localAgent";

// Each chat should keep its own model: switching models in one conversation
// must not change what another conversation runs. Before the fix a single
// global localStorage setting was the only model, so the composer pill in any
// chat edited the one default every chat displayed and baked into its config.

const baseSettings = {
  cwd: "/tmp/project",
  model: "clark-code",
  reasoningEffort: "",
  apiKey: "",
};

const sessionA = { id: "chat-a", provider: "local" } as unknown as Session;
const sessionB = { id: "chat-b", provider: "local" } as unknown as Session;

function stubBridge(overrides: Partial<CoreBridge> = {}): CoreBridge {
  return {
    listProviders: async () => [{ id: "local", label: "Local", capabilities: {
      streaming: true, permissions: true, fs: true, terminal: true, load_session: false, modes: [],
    } }],
    connect: vi.fn(async () => {}),
    newSession: vi.fn(async (_providerId: string, _options: SessionOptions, bindId?: string) =>
      bindId ? { ...sessionA, id: bindId } : sessionA,
    ),
    loadSession: async () => sessionA,
    prompt: async () => {},
    cancel: vi.fn(async () => {}),
    respond: vi.fn(async () => {}),
    setMode: vi.fn(async () => {}),
    subscribe: () => () => {},
    ...overrides,
  } as unknown as CoreBridge;
}

beforeEach(() => {
  useSessionStore.setState({
    bridge: null,
    session: null,
    snapshot: emptySnapshot(),
    approvalPolicy: "auto",
    activeProvider: "local",
    providers: [],
    auth: null,
    connecting: false,
    opening: null,
    queued: [],
    conversations: [],
    localSettings: { ...baseSettings },
    chatModels: {},
    activeRemote: null,
  });
});

describe("per-conversation model", () => {
  it("changing the model in one chat does not affect another", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, localSettings: { ...baseSettings }, chatModels: {} });

    // Chat A: switch to Grok 4.5.
    useSessionStore.setState({ session: sessionA });
    await useSessionStore.getState().updateModelSettings({ model: "clark-code:grok45" });

    // Chat B never diverges: its effective model is still the global default.
    useSessionStore.setState({ session: sessionB });
    expect(
      effectiveModelSettings(useSessionStore.getState().localSettings, useSessionStore.getState().chatModels, sessionB.id).model,
    ).toBe("clark-code");

    // Chat A keeps its own choice.
    expect(
      effectiveModelSettings(useSessionStore.getState().localSettings, useSessionStore.getState().chatModels, sessionA.id).model,
    ).toBe("clark-code:grok45");

    // The global default the start screen shows is untouched — only the chat
    // override moved.
    expect(useSessionStore.getState().localSettings.model).toBe("clark-code");
  });

  it("pins a new chat to the model it was created with", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({
      bridge,
      providers: await bridge.listProviders(),
      localSettings: { ...baseSettings },
      chatModels: {},
      projectMode: "local",
    });

    await useSessionStore.getState().startSession();

    expect(useSessionStore.getState().chatModels[sessionA.id]).toEqual({
      model: "clark-code",
      reasoningEffort: "",
    });

    // The picker on the start screen edits the default for the NEXT chat. It
    // must not retroactively change a conversation that already exists.
    useSessionStore.getState().endSession();
    await useSessionStore.getState().updateModelSettings({ model: "clark-code:kimi_k3" });
    const state = useSessionStore.getState();
    expect(
      effectiveModelSettings(state.localSettings, state.chatModels, sessionA.id),
    ).toMatchObject({ model: "clark-code", reasoningEffort: "" });
  });

  it("pins an existing untracked chat when it is reopened", async () => {
    const bridge = stubBridge();
    const legacyId = "legacy-chat-without-model-settings";
    useSessionStore.setState({
      bridge,
      providers: await bridge.listProviders(),
      localSettings: { ...baseSettings },
      chatModels: {},
      conversations: [
        {
          id: legacyId,
          title: "Legacy chat",
          provider: "local",
          project: baseSettings.cwd,
          createdAt: 1,
          updatedAt: 1,
        },
      ],
    });

    await useSessionStore.getState().openConversation(legacyId);
    expect(useSessionStore.getState().chatModels[legacyId]).toEqual({
      model: "clark-code",
      reasoningEffort: "",
    });

    useSessionStore.getState().endSession();
    await useSessionStore.getState().updateModelSettings({ model: "clark-code:grok45" });
    const state = useSessionStore.getState();
    expect(effectiveModelSettings(state.localSettings, state.chatModels, legacyId).model).toBe(
      "clark-code",
    );
  });

  it("the per-chat model overrides the global default", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, localSettings: { ...baseSettings }, chatModels: {} });

    useSessionStore.setState({ session: sessionA });
    await useSessionStore.getState().updateModelSettings({ model: "clark-code:grok45" });
    await useSessionStore.getState().updateModelSettings({ reasoningEffort: "high" });

    useSessionStore.setState({ session: sessionB });
    await useSessionStore.getState().updateModelSettings({ model: "clark-code:kimi_k3" });

    const { chatModels, localSettings } = useSessionStore.getState();
    expect(effectiveModelSettings(localSettings, chatModels, sessionA.id)).toMatchObject({
      model: "clark-code:grok45",
      reasoningEffort: "high",
    });
    expect(effectiveModelSettings(localSettings, chatModels, sessionB.id).model).toBe("clark-code:kimi_k3");
    expect(effectiveModelSettings(localSettings, chatModels, sessionB.id).reasoningEffort).toBe("max");
  });

  it("normalizes the effort atomically when switching model contracts", async () => {
    const reconfigure = vi.fn(async () => {});
    const bridge = stubBridge({ reconfigure });
    useSessionStore.setState({
      bridge,
      session: sessionA,
      localSettings: { ...baseSettings, reasoningEffort: "xhigh" },
      chatModels: {},
    });

    await useSessionStore.getState().updateModelSettings({ model: "clark-code:kimi_k3" });

    expect(useSessionStore.getState().chatModels[sessionA.id]).toEqual({
      model: "clark-code:kimi_k3",
      reasoningEffort: "max",
    });
    const calls = vi.mocked(reconfigure).mock.calls as unknown as [string, { extra?: Record<string, unknown> }][];
    expect(calls[0]?.[1].extra).toMatchObject({
      model: "clark-code:kimi_k3",
      reasoning_effort: "max",
    });
  });

  it("with no active chat, updating the model edits the global default", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, session: null, localSettings: { ...baseSettings }, chatModels: {} });

    await useSessionStore.getState().updateModelSettings({ model: "clark-code:grok45" });

    // Start-screen picker (no chat) edits the default new chats seed from — no
    // per-chat override is written.
    expect(useSessionStore.getState().localSettings.model).toBe("clark-code:grok45");
    expect(Object.keys(useSessionStore.getState().chatModels)).toHaveLength(0);
  });

  it("reconfigures the live provider with the chat's effective model", async () => {
    const reconfigure = vi.fn(async () => {});
    const bridge = stubBridge({ reconfigure });
    useSessionStore.setState({ bridge, localSettings: { ...baseSettings }, chatModels: {} });

    useSessionStore.setState({ session: sessionA });
    await useSessionStore.getState().updateModelSettings({ model: "clark-code:grok45" });

    expect(reconfigure).toHaveBeenCalledTimes(1);
    const calls = vi.mocked(reconfigure).mock.calls as unknown as [string, { extra?: unknown }][];
    const configArg = calls[0]?.[1];
    expect(configArg?.extra).toMatchObject({ model: "clark-code:grok45" });
  });
});
