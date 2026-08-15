import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionStore } from "./sessionStore";
import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot, type Session } from "../core-bridge/types";
import {
  composerDraftRef,
  composerDraftOwner,
  loadComposerDraft,
  saveComposerDraft,
  specialistStartComposerDraftId,
} from "../lib/composerDraft";
import { DEFAULT_LOCAL_SETTINGS, effectiveModelSettings } from "../lib/localAgent";
import { liveSessions, newLiveEntry } from "./sessionStore.runtime";
import { useSpecialistStore } from "./specialistStore";

// Each chat should keep its own model: switching models in one conversation
// must not change what another conversation runs. Before the fix a single
// global localStorage setting was the only model, so the composer pill in any
// chat edited the one default every chat displayed and baked into its config.

const baseSettings = {
  ...DEFAULT_LOCAL_SETTINGS,
  cwd: "/tmp/project",
};

const sessionA = { id: "chat-a", provider: "local" } as unknown as Session;
const sessionB = { id: "chat-b", provider: "local" } as unknown as Session;
const originalSpecialistOpen = useSpecialistStore.getState().open;

function stubBridge(overrides: Partial<CoreBridge> = {}): CoreBridge {
  return {
    listProviders: async () => [{ id: "local", label: "Local", capabilities: {
      streaming: true, permissions: true, fs: true, terminal: true, load_session: false, modes: [],
    } }],
    openSession: vi.fn(async (_providerId, _config, request) =>
      request.kind === "new" && request.bindId ? { ...sessionA, id: request.bindId } : sessionA),
    prompt: async () => {},
    cancel: vi.fn(async () => {}),
    respond: vi.fn(async () => {}),
    setMode: vi.fn(async () => {}),
    subscribe: () => () => {},
    ...overrides,
  } as unknown as CoreBridge;
}

beforeEach(() => {
  liveSessions.clear();
  localStorage.clear();
  useSpecialistStore.setState({ open: originalSpecialistOpen });
  useSpecialistStore.getState().close();
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
  it("keeps an active composer draft scoped to its conversation when detaching", () => {
    const owner = composerDraftOwner(null);
    saveComposerDraft(owner, sessionA.id, "keep this draft");
    useSessionStore.setState({ session: sessionA });

    useSessionStore.getState().endSession();

    expect(useSessionStore.getState().composerPrefill).toBeNull();
    expect(loadComposerDraft(owner, sessionA.id)).toBe("keep this draft");
  });

  it("does not carry a composer draft through forced sign-out", () => {
    const owner = composerDraftOwner(null);
    saveComposerDraft(owner, sessionA.id, "discard this draft");
    useSessionStore.setState({ session: sessionA });

    useSessionStore.getState().endSession({ force: true });

    expect(useSessionStore.getState().composerPrefill).toBeNull();
  });

  it("clears an abandoned specialist start draft when starting a new session", () => {
    const owner = composerDraftOwner(null);
    const draftId = specialistStartComposerDraftId("spec");
    saveComposerDraft(owner, draftId, "unsent Spec request");
    useSpecialistStore.setState({
      active: "spec",
      contexts: { spec: { kind: "spec" } },
    });

    useSessionStore.getState().endSession();

    expect(loadComposerDraft(owner, draftId)).toBe("");
    expect(useSpecialistStore.getState().active).toBeNull();
  });

  it("keeps the regular New session draft behind when opening a specialist conversation", async () => {
    const bridge = stubBridge({
      projectContext: async (cwd) => ({
        branch: "main",
        detached: false,
        isWorktree: false,
        worktreeRoot: cwd,
        activity: {
          changedFiles: 0,
          untrackedFiles: 0,
          conflictedFiles: 0,
          externalAgents: [],
          detectedAtMs: 1,
        },
      }),
    });
    const securitySession = {
      ...sessionA,
      id: "security-chat",
    } as Session;
    const owner = composerDraftOwner(null);
    const sentinel = "REGULAR NEW SESSION ONLY";
    saveComposerDraft(owner, null, sentinel);
    saveComposerDraft(owner, securitySession.id, "");
    composerDraftRef.current = sentinel;
    useSessionStore.setState({
      bridge,
      providers: await bridge.listProviders(),
      conversations: [{
        id: securitySession.id,
        title: "Security review",
        provider: "local",
        project: baseSettings.cwd,
        createdAt: 1,
        updatedAt: 1,
        specialist: { kind: "security" },
      }],
      composerPrefill: { text: sentinel },
    });
    useSpecialistStore.setState({
      open: (kind, context = {}) => useSpecialistStore.setState((state) => ({
        active: kind,
        expanded: kind,
        contexts: {
          ...state.contexts,
          [kind]: { ...state.contexts[kind], ...context, kind },
        },
      })),
    });

    const specialistOpenObservations: Array<{ active: string | null; session: string | null }> = [];
    const unsubscribe = useSpecialistStore.subscribe((state) => {
      specialistOpenObservations.push({
        active: state.active,
        session: useSessionStore.getState().session?.id ?? null,
      });
    });
    expect(useSessionStore.getState()).toMatchObject({
      activeProvider: "local",
      session: null,
      opening: null,
    });
    expect(useSessionStore.getState().bridge).toBe(bridge);
    await useSessionStore.getState().openConversation(securitySession.id);
    unsubscribe();

    expect(useSessionStore.getState().session?.id).toBe(securitySession.id);
    expect(specialistOpenObservations).toContainEqual({
      active: "security",
      session: securitySession.id,
    });
    expect(useSessionStore.getState().composerPrefill).toBeNull();
    expect(composerDraftRef.current).toBe("");
    expect(loadComposerDraft(owner, securitySession.id)).toBe("");
    expect(loadComposerDraft(owner, null)).toBe(sentinel);
  });

  it("changing the model in one chat does not affect another", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, localSettings: { ...baseSettings }, chatModels: {} });

    // Chat A: switch to the included model.
    useSessionStore.setState({ session: sessionA });
    await useSessionStore.getState().updateModelSettings({ model: "local-model" });

    // Chat B never diverges: its effective model is still the global default.
    useSessionStore.setState({ session: sessionB });
    expect(
      effectiveModelSettings(useSessionStore.getState().localSettings, useSessionStore.getState().chatModels, sessionB.id).model,
    ).toBe(DEFAULT_LOCAL_SETTINGS.model);

    // Chat A keeps its own choice.
    expect(
      effectiveModelSettings(useSessionStore.getState().localSettings, useSessionStore.getState().chatModels, sessionA.id).model,
    ).toBe("local-model");

    // The global default the start screen shows is untouched — only the chat
    // override moved.
    expect(useSessionStore.getState().localSettings.model).toBe(DEFAULT_LOCAL_SETTINGS.model);
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

    expect(vi.mocked(bridge.openSession).mock.calls[0]?.[2]).toMatchObject({
      kind: "new",
      options: { mode: "auto", collaboration_mode: "default" },
    });
    expect(useSessionStore.getState().chatModels[sessionA.id]).toEqual({
      model: DEFAULT_LOCAL_SETTINGS.model,
      reasoningEffort: DEFAULT_LOCAL_SETTINGS.reasoningEffort,
    });

    // The picker on the start screen edits the default for the NEXT chat. It
    // must not retroactively change a conversation that already exists.
    useSessionStore.getState().endSession();
    await useSessionStore.getState().updateModelSettings({ model: "local-model-large" });
    const state = useSessionStore.getState();
    expect(
      effectiveModelSettings(state.localSettings, state.chatModels, sessionA.id),
    ).toMatchObject({
      model: DEFAULT_LOCAL_SETTINGS.model,
      reasoningEffort: DEFAULT_LOCAL_SETTINGS.reasoningEffort,
    });
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
    expect(vi.mocked(bridge.openSession).mock.calls[0]?.[2]).toMatchObject({
      kind: "new",
      options: { mode: "auto", collaboration_mode: "default" },
    });
    expect(useSessionStore.getState().chatModels[legacyId]).toEqual({
      model: DEFAULT_LOCAL_SETTINGS.model,
      reasoningEffort: DEFAULT_LOCAL_SETTINGS.reasoningEffort,
    });

    useSessionStore.getState().endSession();
    await useSessionStore.getState().updateModelSettings({ model: "local-model" });
    const state = useSessionStore.getState();
    expect(effectiveModelSettings(state.localSettings, state.chatModels, legacyId).model).toBe(
      DEFAULT_LOCAL_SETTINGS.model,
    );
  });

  it("the per-chat model overrides the global default", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, localSettings: { ...baseSettings }, chatModels: {} });

    useSessionStore.setState({ session: sessionA });
    await useSessionStore.getState().updateModelSettings({ model: "local-model" });

    useSessionStore.setState({ session: sessionB });
    await useSessionStore.getState().updateModelSettings({ model: "local-model-large" });

    const { chatModels, localSettings } = useSessionStore.getState();
    expect(effectiveModelSettings(localSettings, chatModels, sessionA.id)).toMatchObject({
      model: "local-model",
      reasoningEffort: "high",
    });
    expect(effectiveModelSettings(localSettings, chatModels, sessionB.id).model).toBe("local-model-large");
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

    await useSessionStore.getState().updateModelSettings({ model: "local-model-large" });

    expect(useSessionStore.getState().chatModels[sessionA.id]).toEqual({
      model: "local-model-large",
      reasoningEffort: "max",
    });
    const calls = vi.mocked(reconfigure).mock.calls as unknown as [string, { extra?: Record<string, unknown> }][];
    expect(calls[0]?.[1].extra).toMatchObject({
      model: "local-model-large",
      reasoning_effort: "max",
    });
  });

  it("with no active chat, updating the model edits the global default", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, session: null, localSettings: { ...baseSettings }, chatModels: {} });

    await useSessionStore.getState().updateModelSettings({ model: "local-model" });

    // Start-screen picker (no chat) edits the default new chats seed from — no
    // per-chat override is written.
    expect(useSessionStore.getState().localSettings.model).toBe("local-model");
    expect(Object.keys(useSessionStore.getState().chatModels)).toHaveLength(0);
  });

  it("reconfigures the live provider with the chat's effective model", async () => {
    const reconfigure = vi.fn(async () => {});
    const bridge = stubBridge({ reconfigure });
    useSessionStore.setState({ bridge, localSettings: { ...baseSettings }, chatModels: {} });

    useSessionStore.setState({ session: sessionA });
    await useSessionStore.getState().updateModelSettings({ model: "local-model" });

    expect(reconfigure).toHaveBeenCalledTimes(1);
    const calls = vi.mocked(reconfigure).mock.calls as unknown as [string, { extra?: unknown }][];
    const configArg = calls[0]?.[1];
    expect(configArg?.extra).toMatchObject({ model: "local-model" });
  });

  it("does not mutate a live provider while its run is active", async () => {
    const reconfigure = vi.fn(async () => {});
    const bridge = stubBridge({ reconfigure });
    liveSessions.set(sessionA.id, newLiveEntry(sessionA, {
      historyPrefix: null,
      remote: null,
      remoteHost: null,
      projectRoot: "/tmp/project",
    }));
    useSessionStore.setState({
      bridge,
      session: sessionA,
      snapshot: {
        ...emptySnapshot(),
        session: sessionA.id,
        runs: { "run-1": { id: "run-1", status: "running" } },
      },
    });

    await useSessionStore.getState().updateModelSettings({ model: "local-model-large" });

    expect(reconfigure).not.toHaveBeenCalled();
    expect(useSessionStore.getState().chatModels[sessionA.id]).toBeUndefined();
  });

  it("queues a prompt while model reconfiguration is in flight", async () => {
    let release!: () => void;
    const reconfigureGate = new Promise<void>((resolve) => { release = resolve; });
    const reconfigure = vi.fn(async () => reconfigureGate);
    const prompt = vi.fn(async () => ({ runId: "run-1" }));
    const bridge = stubBridge({ reconfigure, prompt });
    liveSessions.set(sessionA.id, newLiveEntry(sessionA, {
      historyPrefix: null,
      remote: null,
      remoteHost: null,
      projectRoot: "/tmp/project",
    }));
    useSessionStore.setState({ bridge, session: sessionA, snapshot: emptySnapshot() });

    const switching = useSessionStore.getState().updateModelSettings({ model: "local-model-large" });
    await Promise.resolve();
    await useSessionStore.getState().send("hi");

    expect(prompt).not.toHaveBeenCalled();
    expect(useSessionStore.getState().queued.map((message) => message.text)).toEqual(["hi"]);
    release();
    await switching;
    expect(prompt).toHaveBeenCalledTimes(1);
    expect(useSessionStore.getState().queued).toEqual([]);
    expect(liveSessions.get(sessionA.id)?.reconfiguring).toBe(false);
  });
});
