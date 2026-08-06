import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot, type Session } from "../core-bridge/types";
import {
  composerDraftOwner,
  loadComposerDraft,
  saveComposerDraft,
} from "../lib/composerDraft";
import { liveSessions, snapshotCache } from "./sessionStore.runtime";
import { useSessionStore } from "./sessionStore";

const previousSession = { id: "previous-chat", provider: "local" } as Session;
const originalDeleteConversation = useSessionStore.getState().deleteConversation;

function failingBridge(): CoreBridge {
  return {
    openSession: vi.fn(async () => {
      throw new Error("resume transport unavailable");
    }),
    closeSession: vi.fn(async () => {}),
  } as unknown as CoreBridge;
}

beforeEach(() => {
  liveSessions.clear();
  snapshotCache.clear();
  localStorage.clear();
  useSessionStore.setState({
    bridge: failingBridge(),
    activeProvider: "local",
    providers: [{
      id: "local",
      label: "Local",
      capabilities: {
        streaming: true,
        permissions: true,
        fs: true,
        terminal: true,
        load_session: false,
        modes: [],
        collaboration_modes: ["default", "plan"],
      },
    }],
    auth: null,
    session: previousSession,
    snapshot: emptySnapshot(),
    connecting: false,
    opening: null,
    unavailableConversation: null,
    unavailableCleanupId: null,
    error: null,
    conversations: [{
      id: "missing-chat",
      title: "Unavailable work",
      provider: "local",
      project: "/tmp/missing-project",
      createdAt: 1,
      updatedAt: 1,
    }],
    localSettings: {
      cwd: "/tmp/previous-project",
      model: "clark-code",
      reasoningEffort: "",
    },
    chatModels: {},
    attachments: [],
    queued: [],
    historyPrefix: null,
    activeRemote: null,
    activeRemoteHost: null,
    activeProjectRoot: "/tmp/previous-project",
    selectedConversationIds: new Set(),
    mutatingConversationIds: new Set(),
    conversationMutation: null,
    deleteConversation: originalDeleteConversation,
  });
});

afterEach(() => {
  useSessionStore.setState({ deleteConversation: originalDeleteConversation });
});

describe("unavailable conversation navigation", () => {
  it("renders the cached target while native reattachment remains in flight", async () => {
    let finishOpen!: (session: Session) => void;
    const openGate = new Promise<Session>((resolve) => {
      finishOpen = resolve;
    });
    const bridge = failingBridge();
    bridge.openSession = vi.fn(() => openGate);
    const cached = {
      ...emptySnapshot(),
      artifacts: [{ id: "cached-artifact", title: "Cached result", kind: "file" as const }],
    };
    snapshotCache.set("missing-chat", cached);
    useSessionStore.setState({ bridge });

    const opening = useSessionStore.getState().openConversation("missing-chat");
    await vi.waitFor(() => expect(bridge.openSession).toHaveBeenCalled());

    const reconnecting = useSessionStore.getState();
    expect(reconnecting.connecting).toBe(true);
    expect(reconnecting.opening?.id).toBe("missing-chat");
    expect(reconnecting.session?.id).toBe("missing-chat");
    expect(reconnecting.snapshot.artifacts[0]?.id).toBe("cached-artifact");

    finishOpen({
      id: "missing-chat",
      provider: "local",
      capabilities: reconnecting.providers[0]!.capabilities,
      collaboration_mode: "default",
    });
    await opening;
    expect(useSessionStore.getState().connecting).toBe(false);
  });

  it("keeps the failed target selected instead of restoring the previous chat", async () => {
    await useSessionStore.getState().openConversation("missing-chat");

    const state = useSessionStore.getState();
    expect(state.session).toBeNull();
    expect(state.unavailableConversation).toEqual({
      id: "missing-chat",
      title: "Unavailable work",
      detail: "Error: resume transport unavailable",
      kind: "unavailable",
    });
    expect(state.opening).toBeNull();
    expect(state.error).toBeNull();
    expect(state.activeProjectRoot).toBeNull();
  });

  it("cleans up the unavailable entry and resets the new-chat composer", async () => {
    const owner = composerDraftOwner(null);
    saveComposerDraft(owner, null, "draft tied to the old project");
    useSessionStore.setState({
      session: null,
      unavailableConversation: {
        id: "missing-chat",
        title: "Unavailable work",
        detail: "missing",
        kind: "unavailable",
      },
    });
    const draftsWhenComposerCanRemount: string[] = [];
    const unsubscribe = useSessionStore.subscribe((state) => {
      if (!state.unavailableConversation) {
        draftsWhenComposerCanRemount.push(loadComposerDraft(owner, null));
      }
    });

    await useSessionStore.getState().cleanupUnavailableConversation();
    unsubscribe();

    const state = useSessionStore.getState();
    expect(state.conversations).toEqual([]);
    expect(state.unavailableConversation).toBeNull();
    expect(state.session).toBeNull();
    expect(state.localSettings.cwd).toBe("");
    expect(state.activeProjectRoot).toBeNull();
    expect(state.projectMode).toBe("local");
    expect(loadComposerDraft(owner, null)).toBe("");
    expect(draftsWhenComposerCanRemount).toEqual(expect.arrayContaining([""]));
    expect(draftsWhenComposerCanRemount).not.toContain("draft tied to the old project");
  });

  it("does not let a late cleanup reset navigation that happened while deletion was pending", async () => {
    let finishDelete!: () => void;
    const deleteGate = new Promise<void>((resolve) => {
      finishDelete = resolve;
    });
    useSessionStore.setState({
      session: null,
      unavailableConversation: {
        id: "missing-chat",
        title: "Unavailable work",
        detail: "missing",
        kind: "unavailable",
      },
      deleteConversation: vi.fn(async (id: string) => {
        await deleteGate;
        useSessionStore.setState((state) => ({
          conversations: state.conversations.filter((conversation) => conversation.id !== id),
        }));
      }),
    });

    const cleanup = useSessionStore.getState().cleanupUnavailableConversation();
    await Promise.resolve();
    useSessionStore.getState().endSession();
    useSessionStore.setState({
      session: { id: "new-chat", provider: "local" } as Session,
      localSettings: {
        ...useSessionStore.getState().localSettings,
        cwd: "/tmp/new-project",
      },
      activeProjectRoot: "/tmp/new-project",
    });
    finishDelete();
    await cleanup;

    const state = useSessionStore.getState();
    expect(state.session?.id).toBe("new-chat");
    expect(state.localSettings.cwd).toBe("/tmp/new-project");
    expect(state.activeProjectRoot).toBe("/tmp/new-project");
    expect(state.unavailableCleanupId).toBeNull();
  });

  it("fails closed instead of reopening a native local chat with empty history", async () => {
    const bridge = failingBridge();
    bridge.openSession = vi.fn(async () => ({ id: "missing-chat", provider: "local" }) as Session);
    bridge.configureCloudTrajectory = vi.fn(async () => {});
    useSessionStore.setState({ bridge });

    await useSessionStore.getState().openConversation("missing-chat");

    expect(bridge.openSession).not.toHaveBeenCalled();
    expect(useSessionStore.getState().unavailableConversation?.detail).toContain(
      "did not open an empty replacement",
    );
  });

  it("reopens a saved chat through its original provider instead of the current new-chat choice", async () => {
    const bridge = failingBridge();
    bridge.openSession = vi.fn(async (provider, _config, request) => ({
      id: request.kind === "load" ? request.id : request.bindId ?? "new",
      provider,
    }) as Session);
    useSessionStore.setState({
      bridge,
      activeProvider: "local",
      providers: [
        ...useSessionStore.getState().providers,
        {
          id: "acp",
          label: "ACP",
          capabilities: {
            streaming: true,
            permissions: true,
            fs: true,
            terminal: true,
            load_session: true,
            modes: [],
            collaboration_modes: ["default", "plan"],
          },
        },
      ],
      conversations: [{
        id: "acp-chat",
        title: "ACP work",
        provider: "acp",
        createdAt: 1,
        updatedAt: 1,
      }],
    });

    await useSessionStore.getState().openConversation("acp-chat");

    expect(bridge.openSession).toHaveBeenCalledWith(
      "acp",
      expect.any(Object),
      { kind: "load", id: "acp-chat" },
    );
    expect(useSessionStore.getState().activeProvider).toBe("acp");
    expect(useSessionStore.getState().session?.provider).toBe("acp");
  });
});
