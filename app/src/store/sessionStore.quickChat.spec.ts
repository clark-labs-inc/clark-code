import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CoreBridge, SessionOpenRequest } from "../core-bridge/bridge";
import { emptySnapshot, type Session } from "../core-bridge/types";
import { DEFAULT_LOCAL_SETTINGS } from "../lib/localAgent";
import { isQuickChatProject } from "../lib/projectSidebar";
import { useSessionStore } from "./sessionStore";
import { liveSessions } from "./sessionStore.runtime";

describe("Quick Chat", () => {
  beforeEach(() => {
    liveSessions.clear();
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
      localSettings: {
        ...DEFAULT_LOCAL_SETTINGS,
        cwd: "/repo/remembered-project",
        model: "local-model-large",
        reasoningEffort: "max",
      },
      chatModels: {},
      recentProjects: ["/repo/remembered-project"],
    });
  });

  it("starts in a conversation-bound workspace without changing the remembered project", async () => {
    const openSession = vi.fn(async (
      _provider: string,
      _config: object,
      request: SessionOpenRequest,
    ): Promise<Session> => {
      if (request.kind !== "new") throw new Error("expected new session");
      const { options, bindId } = request;
      const id = bindId ?? "missing-id";
      const root = `/Users/test/.agent/workspace/${id}`;
      return {
        id,
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
        collaboration_mode: options.collaboration_mode ?? "default",
        environment: {
          checkout_root: root,
          workspace_roots: [root],
          docs_root: root,
          remote: false,
        },
      };
    });
    const bridge = {
      prepareQuickChatWorkspace: vi.fn(async () => ({
        id: "912a9700-7f5f-4f18-9785-b5d9315a41b4",
        path: "/Users/test/.agent/workspace/912a9700-7f5f-4f18-9785-b5d9315a41b4",
      })),
      openSession,
      subscribe: () => () => {},
    } as unknown as CoreBridge;
    useSessionStore.setState({ bridge });

    await useSessionStore.getState().startQuickChat();

    expect(openSession).toHaveBeenCalledTimes(1);
    expect(openSession.mock.calls[0]?.[1]).toMatchObject({
      extra: { model: "local-model", reasoning_effort: "high" },
    });
    const request = openSession.mock.calls[0]?.[2];
    expect(request).toMatchObject({
      kind: "new",
      options: {
        cwd: "/Users/test/.agent/workspace/912a9700-7f5f-4f18-9785-b5d9315a41b4",
      },
      bindId: "912a9700-7f5f-4f18-9785-b5d9315a41b4",
    });
    const state = useSessionStore.getState();
    expect(state.localSettings.cwd).toBe("/repo/remembered-project");
    expect(state.recentProjects).toEqual(["/repo/remembered-project"]);
    expect(isQuickChatProject(state.activeProjectRoot ?? undefined, state.session!.id)).toBe(true);
    expect(state.conversations[0]?.project).toBe(state.activeProjectRoot);
    expect(state.chatModels[state.session!.id]).toEqual({
      model: "local-model",
      reasoningEffort: "high",
    });
  });

  it("reopens a cloud-saved Quick Chat under this device's workspace root", async () => {
    const id = "912a9700-7f5f-4f18-9785-b5d9315a41b4";
    const currentPath = `/Users/current/.agent/workspace/${id}`;
    const prepareQuickChatWorkspace = vi.fn(async () => ({ id, path: currentPath }));
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
        checkout_root: currentPath,
        workspace_roots: [currentPath],
        docs_root: currentPath,
        remote: false,
      },
    }});
    const bridge = {
      prepareQuickChatWorkspace,
      openSession,
      subscribe: () => () => {},
    } as unknown as CoreBridge;
    useSessionStore.setState({
      bridge,
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
          collaboration_modes: [],
        },
      }],
      conversations: [{
        id,
        title: "Saved Quick Chat",
        provider: "local",
        project: `/home/previous/.agent/workspace/${id}`,
        createdAt: 1,
        updatedAt: 1,
      }],
      chatModels: {
        [id]: { model: "local-model-large", reasoningEffort: "max" },
      },
    });

    await useSessionStore.getState().openConversation(id);

    expect(prepareQuickChatWorkspace).toHaveBeenCalledWith(id);
    expect(openSession).toHaveBeenCalledWith(
      "local",
      expect.objectContaining({
        extra: expect.objectContaining({
          model: "local-model",
          reasoning_effort: "high",
        }),
      }),
      { kind: "new", options: expect.objectContaining({ cwd: currentPath }), bindId: id },
    );
    expect(useSessionStore.getState().activeProjectRoot).toBe(currentPath);
    expect(useSessionStore.getState().chatModels[id]).toEqual({
      model: "local-model",
      reasoningEffort: "high",
    });
  });
});
