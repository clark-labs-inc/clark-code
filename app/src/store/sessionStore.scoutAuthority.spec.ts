import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { CoreBridge, SessionOpenRequest } from "../core-bridge/bridge";
import { emptySnapshot, type Session } from "../core-bridge/types";
import { DEFAULT_LOCAL_SETTINGS } from "../lib/localAgent";
import { installProductModule, neutralProduct } from "../product/productModule";
import { useSessionStore } from "./sessionStore";
import { liveSessions } from "./sessionStore.runtime";
import { useSpecialistStore } from "./specialistStore";

const organizationId = "59b8fe20-6072-4c16-9dae-9d7cbbf2533c";
const workspaceId = "2fac2db5-20d6-499c-b691-47ad19fc0ca8";

function session(id: string, path: string): Session {
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
    collaboration_mode: "default",
    environment: {
      checkout_root: path,
      workspace_roots: [path],
      docs_root: path,
      remote: false,
    },
  };
}

describe("human-bound Scout authority", () => {
  beforeEach(() => {
    liveSessions.clear();
    installProductModule({
      ...neutralProduct,
      specialistWorkspace: {
        isConversationBound: (kind) => kind === "scout",
      },
    });
    useSpecialistStore.setState({
      active: "scout",
      contexts: {
        scout: { kind: "scout", organizationId, workspaceId },
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
      error: null,
      projectMode: "remote",
      localSettings: { ...DEFAULT_LOCAL_SETTINGS, cwd: "/repo/previous-package" },
      recentProjects: ["/repo/previous-package"],
      approvalPolicy: "ask",
      collaborationMode: "plan",
    });
  });

  afterEach(() => {
    installProductModule(neutralProduct);
    useSpecialistStore.setState({ active: null, contexts: {} });
  });

  it("binds Scout to the selected cloud workspace and a neutral local sandbox", async () => {
    const conversationId = "9d2a96d6-67c5-42e8-84cc-b60fe09e41ac";
    const neutralPath = `/Users/test/.agent/workspace/${conversationId}`;
    const openSession = vi.fn(async (
      _provider: string,
      _config: object,
      request: SessionOpenRequest,
    ) => {
      if (request.kind !== "new") throw new Error("expected new session");
      return session(request.bindId!, neutralPath);
    });
    const bridge = {
      prepareQuickChatWorkspace: vi.fn(async () => ({
        id: conversationId,
        path: neutralPath,
      })),
      openSession,
      subscribe: () => () => {},
    } as unknown as CoreBridge;
    useSessionStore.setState({ bridge });

    await useSessionStore.getState().startSession();

    expect(openSession).toHaveBeenCalledWith(
      "local",
      expect.objectContaining({
        cwd: neutralPath,
        extra: expect.objectContaining({
          scout_cartography: expect.objectContaining({
            organization_id: organizationId,
            workspace_id: workspaceId,
            human_run_request_id: expect.stringMatching(/^scout-run:[0-9a-f]{64}$/),
          }),
        }),
      }),
      {
        kind: "new",
        options: expect.objectContaining({
          cwd: neutralPath,
          mode: "full",
          collaboration_mode: "default",
        }),
        bindId: conversationId,
      },
    );
    expect(useSessionStore.getState().localSettings.cwd).toBe("/repo/previous-package");
    expect(useSessionStore.getState().conversations[0]?.specialist).toMatchObject({
      kind: "scout",
      organizationId,
      workspaceId,
      scoutRunRequestId: expect.stringMatching(/^scout-run:[0-9a-f]{64}$/),
    });
    expect(bridge.prepareQuickChatWorkspace).toHaveBeenCalledOnce();
    expect(openSession.mock.calls[0]?.[1]).not.toHaveProperty("extra.remote_worker");
  });

  it("refuses to infer or create a workspace during run submission", async () => {
    useSpecialistStore.setState({
      contexts: { scout: { kind: "scout", organizationId } },
    });
    const prepareQuickChatWorkspace = vi.fn();
    const openSession = vi.fn();
    useSessionStore.setState({
      bridge: {
        prepareQuickChatWorkspace,
        openSession,
      } as unknown as CoreBridge,
    });

    await useSessionStore.getState().startSession();

    expect(prepareQuickChatWorkspace).not.toHaveBeenCalled();
    expect(openSession).not.toHaveBeenCalled();
    expect(useSessionStore.getState().error).toBe(
      "Choose or create a Scout workspace before starting Scout.",
    );
  });
});
