import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CoreBridge, SessionOpenRequest } from "../core-bridge/bridge";
import { emptySnapshot, type Session } from "../core-bridge/types";
import { DEFAULT_LOCAL_SETTINGS } from "../lib/localAgent";
import {
  composerDraftOwner,
  loadComposerDraft,
  saveComposerDraft,
} from "../lib/composerDraft";
import { installProductModule, neutralProduct } from "../product/productModule";
import { useSessionStore } from "./sessionStore";
import { liveSessions } from "./sessionStore.runtime";
import { useSpecialistStore } from "./specialistStore";

describe("conversation-bound Spec workspace", () => {
  beforeEach(() => {
    liveSessions.clear();
    installProductModule({
      ...neutralProduct,
      specialistWorkspace: {
        isConversationBound: (kind) => kind === "spec",
      },
    });
    useSpecialistStore.setState({
      active: "spec",
      contexts: { spec: { kind: "spec" } },
    });
    useSessionStore.setState({
      bridge: null,
      session: null,
      snapshot: emptySnapshot(),
      activeProvider: "local",
      providers: [],
      auth: null,
      connecting: false,
      opening: null,
      conversations: [],
      localSettings: { ...DEFAULT_LOCAL_SETTINGS, cwd: "/repo/remembered-project" },
      recentProjects: ["/repo/remembered-project"],
    });
  });

  afterEach(() => {
    installProductModule(neutralProduct);
    useSpecialistStore.setState({ active: null, contexts: {} });
  });

  it("runs in a conversation workspace without replacing the remembered repository", async () => {
    const id = "912a9700-7f5f-4f18-9785-b5d9315a41b4";
    const path = `/Users/test/.agent/workspace/${id}`;
    const openSession = vi.fn(async (
      _provider: string,
      _config: object,
      request: SessionOpenRequest,
    ): Promise<Session> => {
      if (request.kind !== "new") throw new Error("expected new session");
      return {
      id: request.bindId!,
      provider: "local",
      capabilities: {
        streaming: true,
        permissions: true,
        fs: true,
        terminal: true,
        load_session: false,
        modes: [],
        collaboration_modes: [],
      },
      collaboration_mode: request.options.collaboration_mode ?? "default",
      environment: {
        checkout_root: path,
        workspace_roots: [path],
        docs_root: path,
        remote: false,
      },
    };
    });
    const bridge = {
      prepareQuickChatWorkspace: vi.fn(async () => ({ id, path })),
      openSession,
      subscribe: () => () => {},
    } as unknown as CoreBridge;
    useSessionStore.setState({ bridge });
    const draftOwner = composerDraftOwner(null);
    saveComposerDraft(draftOwner, null, "ordinary chat draft");

    await useSessionStore.getState().startSession();

    expect(openSession).toHaveBeenCalledWith(
      "local",
      expect.any(Object),
      { kind: "new", options: expect.objectContaining({ cwd: path }), bindId: id },
    );
    const state = useSessionStore.getState();
    expect(state.localSettings.cwd).toBe("/repo/remembered-project");
    expect(state.recentProjects).toEqual(["/repo/remembered-project"]);
    expect(state.activeProjectRoot).toBe(path);
    expect(state.conversations[0]?.specialist?.kind).toBe("spec");
    expect(loadComposerDraft(draftOwner, id)).toBe("");
    expect(loadComposerDraft(draftOwner, null)).toBe("ordinary chat draft");
  });
});
