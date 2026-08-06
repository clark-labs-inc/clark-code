import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CoreBridge } from "../core-bridge/bridge";
import {
  emptySnapshot,
  type ProviderInfo,
  type Session,
} from "../core-bridge/types";
import { useSessionStore } from "./sessionStore";
import { liveSessions } from "./sessionStore.runtime";
import { useSpecialistStore } from "./specialistStore";
import { saveSshHosts } from "../lib/sshHosts";

const projectRoot = "/tmp/scientist-project";

function specialistSession(id = "specialist-session"): Session {
  return {
    id,
    provider: "specialist",
    capabilities: {
      streaming: true,
      permissions: false,
      fs: false,
      terminal: false,
      load_session: true,
      modes: [],
      collaboration_modes: ["default"],
    },
    collaboration_mode: "default",
    environment: {
      checkout_root: projectRoot,
      workspace_roots: [projectRoot],
      remote: false,
    },
  };
}

function bridge(): CoreBridge {
  return {
    listProviders: async () => [],
    openSession: vi.fn(async (_provider, _config, request) =>
      specialistSession(request.kind === "load" ? request.id : request.bindId)),
    closeSession: vi.fn(async () => {}),
    prompt: vi.fn(async () => ({ runId: "run-1" })),
    cancel: vi.fn(async () => {}),
    respond: vi.fn(async () => {}),
    subscribe: () => () => {},
  } as unknown as CoreBridge;
}

const providers: ProviderInfo[] = [
  {
    id: "local",
    label: "Clark Code",
    capabilities: {
      streaming: true,
      permissions: true,
      fs: true,
      terminal: true,
      load_session: false,
      modes: [],
      collaboration_modes: ["default", "plan"],
    },
  },
  {
    id: "specialist",
    label: "Clark Specialist Runtime",
    internal: true,
    capabilities: {
      streaming: true,
      permissions: false,
      fs: false,
      terminal: false,
      load_session: true,
      modes: [],
      collaboration_modes: ["default"],
    },
  },
];

beforeEach(() => {
  liveSessions.clear();
  localStorage.clear();
  useSpecialistStore.getState().close();
  useSessionStore.getState().endSession({ force: true });
  useSessionStore.setState({
    bridge: null,
    session: null,
    snapshot: emptySnapshot(),
    activeProvider: "local",
    providers,
    auth: null,
    connecting: false,
    opening: null,
    unavailableConversation: null,
    unavailableCleanupId: null,
    error: null,
    conversations: [],
    localSettings: {
      cwd: projectRoot,
      model: "clark-code",
      reasoningEffort: "",
    },
    chatModels: {},
    projectMode: "local",
    collaborationMode: "plan",
    activeRemote: null,
    activeRemoteHost: null,
    activeProjectRoot: null,
    attachments: [],
    queued: [],
    historyPrefix: null,
  });
});

describe("research specialist native routing", () => {
  it("does not allow model or reasoning changes in a specialist session", async () => {
    useSpecialistStore.getState().open("security");
    useSessionStore.setState({ session: specialistSession() });

    await useSessionStore.getState().updateModelSettings({
      model: "clark-code:kimi_k3",
      reasoningEffort: "low",
    });

    expect(useSessionStore.getState().chatModels).toEqual({});
    expect(useSessionStore.getState().localSettings.model).toBe("clark-code");
    expect(useSessionStore.getState().localSettings.reasoningEffort).toBe("");
  });

  it("routes Scientist through the internal provider without creating a worktree", async () => {
    const native = bridge();
    useSessionStore.setState({ bridge: native });
    useSpecialistStore.getState().open("scientist", {
      organizationId: "org-1",
      workflow: "scientist:discover",
    });

    await useSessionStore.getState().startSession();

    expect(native.openSession).toHaveBeenCalledWith("specialist", {
      cwd: projectRoot,
      extra: {
        specialist: "scientist",
        workflow: "scientist:discover",
        organizationId: "org-1",
        modelRoute: "clark_deepseek_v4_latest",
        maxIterations: 3,
      },
    }, {
      kind: "new",
      options: { cwd: projectRoot, collaboration_mode: "default" },
    });
    const state = useSessionStore.getState();
    expect(state.activeProvider).toBe("local");
    expect(state.session?.provider).toBe("specialist");
    expect(state.conversations[0]).toMatchObject({
      provider: "specialist",
      project: projectRoot,
      specialist: {
        kind: "scientist",
        organizationId: "org-1",
        workflow: "scientist:discover",
      },
    });
  });

  it("reopens a durable specialist session through load_session", async () => {
    const native = bridge();
    useSessionStore.setState({
      bridge: native,
      conversations: [{
        id: "saved-specialist",
        title: "Saved discovery",
        provider: "specialist",
        project: projectRoot,
        specialist: {
          kind: "rsi",
          organizationId: "org-1",
          workflow: "rsi:stress-test",
        },
        createdAt: 1,
        updatedAt: 2,
      }],
    });

    await useSessionStore.getState().openConversation("saved-specialist");

    expect(native.openSession).toHaveBeenCalledWith("specialist", expect.objectContaining({
      cwd: projectRoot,
      extra: expect.objectContaining({
        specialist: "rsi",
        workflow: "rsi:stress-test",
      }),
    }), { kind: "load", id: "saved-specialist" });
    expect(useSessionStore.getState().activeProvider).toBe("local");
    expect(useSessionStore.getState().session?.id).toBe("saved-specialist");
  });

  it("routes remote research specialists to a headless SSH worker", async () => {
    const native = bridge();
    saveSshHosts([{
      id: "remote-1",
      label: "GPU",
      host: "gpu.example",
      remoteRoot: "/workspace/project",
    }], null);
    useSpecialistStore.getState().open("rsi", {
      organizationId: "org-1",
      workflow: "rsi:stress-test",
    });
    useSessionStore.setState({
      bridge: native,
      projectMode: "remote",
      selectedHostId: "remote-1",
    });

    expect(useSessionStore.getState().startBlockedReason()).toBeNull();
    await useSessionStore.getState().startSession();

    expect(native.openSession).toHaveBeenCalledWith("specialist", expect.objectContaining({
      cwd: "/workspace/project",
      extra: expect.objectContaining({
        specialist: "rsi",
        workflow: "rsi:stress-test",
        remote: {
          host: "gpu.example",
          remoteRoot: "/workspace/project",
        },
      }),
    }), {
      kind: "new",
      options: { cwd: "/workspace/project", collaboration_mode: "default" },
    });
  });
});
