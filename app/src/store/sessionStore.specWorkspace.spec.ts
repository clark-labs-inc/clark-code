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
import { saveSshHosts } from "../lib/sshHosts";
import { useSessionStore } from "./sessionStore";
import { liveSessions } from "./sessionStore.runtime";
import { useSpecialistStore } from "./specialistStore";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

describe("conversation-bound specialist workspace", () => {
  beforeEach(() => {
    liveSessions.clear();
    localStorage.clear();
    invoke.mockReset();
    invoke.mockResolvedValue({
      id: "worker-scout-1",
      cwd: "/srv/enterprise/project",
      arch: "linux-x86_64",
      sshTransport: "control_master",
      connectionKind: "started",
      connectDurationMs: 42,
      accountWorkerCount: 1,
    });
    installProductModule({
      ...neutralProduct,
      specialistWorkspace: {
        isConversationBound: (kind) => kind === "scout",
      },
    });
    useSpecialistStore.setState({
      active: "scout",
      contexts: {
        scout: {
          kind: "scout",
          organizationId: "018f8e8a-4722-7c68-b5b7-a4c6793c85b0",
          workspaceId: "028f8e8a-4722-7c68-b5b7-a4c6793c85b0",
        },
      },
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
      projectMode: "local",
      selectedHostId: null,
      activeRemote: null,
      activeRemoteHost: null,
      activeProjectRoot: null,
      error: null,
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
    expect(state.conversations[0]?.specialist?.kind).toBe("scout");
    expect(loadComposerDraft(draftOwner, id)).toBe("");
    expect(loadComposerDraft(draftOwner, null)).toBe("ordinary chat draft");
  });

  it("runs Scout on the selected SSH project while retaining its conversation workspace", async () => {
    const id = "remote-scout-workspace";
    const documentPath = `/Users/test/.agent/workspace/${id}`;
    saveSshHosts([{
      id: "enterprise-host",
      label: "Enterprise machine",
      host: "ubuntu@enterprise",
      remoteRoot: "/srv/enterprise/project",
    }], null);
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
          checkout_root: "/srv/enterprise/project",
          workspace_roots: ["/srv/enterprise/project"],
          docs_root: "/srv/enterprise/project",
          remote: true,
        },
      };
    });
    const bridge = {
      prepareQuickChatWorkspace: vi.fn(async () => ({ id, path: documentPath })),
      openSession,
      subscribe: () => () => {},
    } as unknown as CoreBridge;
    useSessionStore.setState({
      bridge,
      projectMode: "remote",
      selectedHostId: "enterprise-host",
    });

    await useSessionStore.getState().startSession();

    expect(invoke).toHaveBeenCalledWith("remote_worker_connect", {
      input: expect.objectContaining({
        host: "ubuntu@enterprise",
        remoteRoot: "/srv/enterprise/project",
      }),
    });
    expect(openSession).toHaveBeenCalledWith(
      "local",
      {
        extra: {
          remote_worker: {
            worker_handle: "worker-scout-1",
            cwd: "/srv/enterprise/project",
          },
          specialist_kind: "scout",
          scout_cartography: expect.objectContaining({
            organization_id: "018f8e8a-4722-7c68-b5b7-a4c6793c85b0",
            workspace_id: "028f8e8a-4722-7c68-b5b7-a4c6793c85b0",
          }),
        },
      },
      expect.objectContaining({
        kind: "new",
        options: {
          cwd: "/srv/enterprise/project",
          mode: "full",
          collaboration_mode: "default",
        },
        bindId: id,
      }),
    );
    expect(bridge.prepareQuickChatWorkspace).toHaveBeenCalledTimes(1);
    expect(useSessionStore.getState()).toMatchObject({
      activeRemoteHost: "ubuntu@enterprise",
      activeProjectRoot: "/srv/enterprise/project",
      conversations: [expect.objectContaining({
        id,
        provider: "local",
        project: "/srv/enterprise/project",
        remoteHost: "ubuntu@enterprise",
        specialist: expect.objectContaining({ kind: "scout" }),
      })],
    });
  });
});
