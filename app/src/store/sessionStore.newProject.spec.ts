import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionStore } from "./sessionStore";
import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot, type Session } from "../core-bridge/types";
import { loadSshHosts, type SshHost } from "../lib/sshHosts";
import { liveSessions } from "./sessionStore.runtime";
import { DEFAULT_LOCAL_SETTINGS } from "../lib/localAgent";

// "New project" (startNewProject): choose a destination — a local folder or a
// remote SSH host + folder — and start its FIRST session immediately instead of
// waiting for a typed prompt. The old openProjectTerminal (folder-without-
// session, used by worktree flows) is a separate, untouched action.

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const baseSettings = {
  ...DEFAULT_LOCAL_SETTINGS,
  cwd: "/tmp/project",
};

const sessionA = { id: "chat-a", provider: "local" } as unknown as Session;

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

const remoteConnection = {
  id: "worker-1",
  cwd: "/remote/root",
  arch: "linux-x86_64",
  sshTransport: "control_master" as const,
  connectionKind: "started" as const,
  connectDurationMs: 42,
  accountWorkerCount: 1,
};

const remoteHost: SshHost = {
  id: "h1",
  label: "GPU box",
  host: "user@box",
  remoteRoot: "/remote/root",
};

beforeEach(() => {
  liveSessions.clear();
  localStorage.clear();
  invoke.mockReset();
  invoke.mockResolvedValue(remoteConnection);
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
    approvalPolicies: {},
    projectMode: "local",
    selectedHostId: null,
    activeRemote: null,
    activeRemoteHost: null,
    activeProjectRoot: null,
    recentProjects: [],
    error: null,
    notice: null,
    runningIds: [],
    newProjectOpen: false,
  });
});

describe("startNewProject (local)", () => {
  it("sets the folder and auto-starts the first local session", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, providers: await bridge.listProviders() });

    await useSessionStore.getState().startNewProject({ kind: "local", path: "/tmp/new-app", base: "current" });

    const s = useSessionStore.getState();
    expect(s.localSettings.cwd).toBe("/tmp/new-app");
    expect(s.projectMode).toBe("local");
    expect(s.activeProvider).toBe("local");
    expect(s.session).not.toBeNull();
    expect(s.connecting).toBe(false);
    expect(s.opening).toBeNull();
    expect(s.activeProjectRoot).toBe("/tmp/new-app");
    expect(s.recentProjects[0]).toBe("/tmp/new-app");
    expect(vi.mocked(bridge.openSession)).toHaveBeenCalledTimes(1);
    expect(vi.mocked(bridge.openSession).mock.calls[0]?.[2]).toMatchObject({ kind: "new" });
  });

  it("no-ops on a blank local path", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, providers: await bridge.listProviders() });

    await useSessionStore.getState().startNewProject({ kind: "local", path: "   ", base: "current" });

    const s = useSessionStore.getState();
    expect(s.localSettings.cwd).toBe("/tmp/project");
    expect(s.session).toBeNull();
    expect(vi.mocked(bridge.openSession)).not.toHaveBeenCalled();
  });

  it("detaches a running session on another root (kept in the pool)", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({
      bridge,
      providers: await bridge.listProviders(),
      session: { ...sessionA } as Session,
      activeProjectRoot: "/tmp/old-app",
      runningIds: ["chat-a"],
    });
    // A live entry already in the pool stays untouched.
    liveSessions.set("chat-a", {
      session: sessionA,
      live: emptySnapshot(),
      historyPrefix: null,
      remote: null,
      remoteHost: null,
      projectRoot: "/tmp/old-app",
      queued: [],
      lastPersist: 0,
      prevBusy: false,
      dispatching: false,
      starting: false,
      reconfiguring: false,
      lastSubmittedText: null,
      lastSubmittedAt: 0,
      autoResolvedId: null,
      notifiedPermId: null,
    });

    await useSessionStore.getState().startNewProject({ kind: "local", path: "/tmp/new-app", base: "current" });

    const s = useSessionStore.getState();
    expect(s.localSettings.cwd).toBe("/tmp/new-app");
    expect(s.session?.id).toBe("chat-a"); // the newly started session
    expect(s.activeProjectRoot).toBe("/tmp/new-app");
    expect(liveSessions.get("chat-a")).not.toBeUndefined(); // old one still running
  });
});

describe("startNewProject (remote SSH)", () => {
  it("saves the host and auto-starts the first remote session", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, providers: await bridge.listProviders() });

    await useSessionStore.getState().startNewProject({ kind: "remote", host: remoteHost });

    const s = useSessionStore.getState();
    expect(s.projectMode).toBe("remote");
    expect(s.selectedHostId).toBe("h1");
    expect(s.activeProvider).toBe("local");
    expect(s.activeRemoteHost).toBe("user@box");
    expect(s.session).not.toBeNull();
    expect(s.connecting).toBe(false);
    expect(s.opening).toBeNull();
    // The chosen host (with its remote folder) was persisted.
    const saved = loadSshHosts(null);
    expect(saved.some((h) => h.id === "h1" && h.remoteRoot === "/remote/root")).toBe(true);
    expect(invoke).toHaveBeenCalledWith("remote_worker_connect", {
      input: {
        host: "user@box",
        remoteRoot: "/remote/root",
        model: DEFAULT_LOCAL_SETTINGS.model,
        reasoningEffort: DEFAULT_LOCAL_SETTINGS.reasoningEffort,
      },
    });
    expect(vi.mocked(bridge.openSession)).toHaveBeenCalledTimes(1);
    expect(vi.mocked(bridge.openSession).mock.calls[0]?.[2]).toMatchObject({ kind: "new" });
  });

  it("rejects a host without an SSH destination or remote folder", async () => {
    const bridge = stubBridge();
    useSessionStore.setState({ bridge, providers: await bridge.listProviders() });

    await useSessionStore.getState().startNewProject({
      kind: "remote",
      host: { id: "h2", label: "", host: "", remoteRoot: "" },
    });

    const s = useSessionStore.getState();
    expect(s.error).toContain("remote folder before starting");
    expect(s.session).toBeNull();
    expect(vi.mocked(bridge.openSession)).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
  });
});
