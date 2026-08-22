import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionStore } from "./sessionStore";
import type {
  CoreBridge,
  ProjectWorktreeTransitionPlan,
} from "../core-bridge/bridge";
import { emptySnapshot, type Session } from "../core-bridge/types";
import {
  composerDraftOwner,
  loadComposerDraft,
  saveComposerDraft,
} from "../lib/composerDraft";

const draftOwner = composerDraftOwner(null);

const sourceRoot = "/tmp/project";
const managedRoot = "/tmp/project.agent-worktrees/session-1";
const session = { id: "managed-chat", provider: "local" } as unknown as Session;

const baseSettings = {
  cwd: sourceRoot,
  model: "local-model",
  reasoningEffort: "",
};

function plan(overrides: Partial<ProjectWorktreeTransitionPlan> = {}): ProjectWorktreeTransitionPlan {
  return {
    sourceRoot,
    sourceBranch: "main",
    sourceRevision: "0123456789abcdef0123456789abcdef01234567",
    sourceChanges: { changedFiles: 0, untrackedFiles: 0, conflictedFiles: 0 },
    sourceIsManaged: false,
    targetBranch: null,
    targetCheckoutPath: null,
    action: "create_isolated",
    preservation: "clean",
    requiresConfirmation: false,
    baseOptions: [
      {
        id: "current",
        label: "Current checkout (main)",
        reference: "main",
        revision: "0123456789abcdef0123456789abcdef01234567",
        fallback: false,
      },
      {
        id: "default",
        label: "Default branch (origin/main)",
        reference: "origin/main",
        revision: "0123456789abcdef0123456789abcdef01234567",
        fallback: false,
      },
    ],
    managedLocation: "/tmp/project.agent-worktrees",
    ...overrides,
  };
}

function bridgeFor(nextPlan: ProjectWorktreeTransitionPlan): CoreBridge {
  return {
    listProviders: async () => [{
      id: "local",
      label: "Local",
      capabilities: {
        streaming: true,
        permissions: true,
        fs: true,
        terminal: true,
        load_session: false,
        modes: [],
      },
    }],
    openSession: vi.fn(async () => session),
    prompt: async () => ({ runId: "run" }),
    cancel: vi.fn(async () => {}),
    respond: vi.fn(async () => {}),
    subscribe: () => () => {},
    projectContext: vi.fn(async () => ({
      branch: "main",
      detached: false,
      isWorktree: false,
      worktreeRoot: sourceRoot,
      activity: {
        changedFiles: 0,
        untrackedFiles: 0,
        conflictedFiles: 0,
        externalAgents: [],
        detectedAtMs: 1,
      },
    })),
    planProjectWorktree: vi.fn(async () => nextPlan),
    createManagedWorktree: vi.fn(async (_root, request) => ({
      id: "session-1",
      label: "session",
      path: managedRoot,
      sourceRoot,
      base: request.base,
      baseReference: request.targetBranch || "main",
      baseRevision: "0123456789abcdef0123456789abcdef01234567",
      createdAtMs: 1,
      state: "ready",
      changes: { changedFiles: 0, untrackedFiles: 0, conflictedFiles: 0 },
    })),
    cleanupManagedWorktree: vi.fn(async (_root, id) => ({
      id,
      path: managedRoot,
      removed: true,
    })),
  } as unknown as CoreBridge;
}

beforeEach(() => {
  useSessionStore.getState().endSession({ force: true });
  saveComposerDraft(draftOwner, session.id, "");
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
    localSettings: { ...baseSettings },
    projectMode: "local",
    activeRemote: null,
    activeRemoteHost: null,
    activeProjectRoot: null,
    managedWorktreeBase: "current",
    worktreeTransition: null,
    pendingManagedWorktreePath: null,
    worktreePreparing: false,
  });
});

describe("managed worktree session journeys", () => {
  it("keeps a clean Git project in the selected checkout by default", async () => {
    const bridge = bridgeFor(plan());
    useSessionStore.setState({ bridge, providers: await bridge.listProviders() });

    await useSessionStore.getState().startSession();

    expect(vi.mocked(bridge.createManagedWorktree)).not.toHaveBeenCalled();
    expect(vi.mocked(bridge.openSession).mock.calls[0]?.[2]).toMatchObject({
      kind: "new",
      options: { cwd: sourceRoot },
    });
    expect(useSessionStore.getState().localSettings.cwd).toBe(sourceRoot);
    expect(useSessionStore.getState().activeProjectRoot).toBe(sourceRoot);
    expect(useSessionStore.getState().conversations[0]?.project).toBe(sourceRoot);
  });

  it("creates an isolated checkout when the user chooses the default branch", async () => {
    const bridge = bridgeFor(plan());
    useSessionStore.setState({
      bridge,
      providers: await bridge.listProviders(),
      managedWorktreeBase: "default",
    });

    await useSessionStore.getState().startSession();

    expect(vi.mocked(bridge.createManagedWorktree)).toHaveBeenCalledWith(sourceRoot, {
      base: "default",
    });
    expect(vi.mocked(bridge.openSession).mock.calls[0]?.[2]).toMatchObject({
      kind: "new",
      options: { cwd: managedRoot },
    });
  });

  it("works in the dirty current checkout without prompting", async () => {
    const bridge = bridgeFor(plan({
      sourceChanges: { changedFiles: 2, untrackedFiles: 1, conflictedFiles: 0 },
      preservation: "changes_remain_in_source",
      requiresConfirmation: true,
    }));
    useSessionStore.setState({ bridge, providers: await bridge.listProviders() });

    await useSessionStore.getState().startSession();

    expect(useSessionStore.getState().worktreeTransition).toBeNull();
    expect(vi.mocked(bridge.createManagedWorktree)).not.toHaveBeenCalled();
    expect(vi.mocked(bridge.openSession).mock.calls[0]?.[2]).toMatchObject({
      kind: "new",
      options: { cwd: sourceRoot },
    });
  });

  it("auto-forks a clean worktree from the default branch on a dirty checkout", async () => {
    const bridge = bridgeFor(plan({
      sourceChanges: { changedFiles: 1, untrackedFiles: 1, conflictedFiles: 0 },
      preservation: "changes_remain_in_source",
      requiresConfirmation: true,
    }));
    useSessionStore.setState({
      bridge,
      providers: await bridge.listProviders(),
      managedWorktreeBase: "default",
    });

    await useSessionStore.getState().startSession();

    expect(useSessionStore.getState().worktreeTransition).toBeNull();
    expect(vi.mocked(bridge.createManagedWorktree)).toHaveBeenCalledWith(sourceRoot, {
      base: "default",
    });
    expect(vi.mocked(bridge.openSession).mock.calls[0]?.[2]).toMatchObject({
      kind: "new",
      options: { cwd: managedRoot },
    });
  });

  it("starts directly in an unborn checkout with untracked files", async () => {
    const bridge = bridgeFor(plan({
      sourceRevision: null,
      sourceChanges: { changedFiles: 0, untrackedFiles: 26, conflictedFiles: 0 },
      preservation: "changes_remain_in_source",
      requiresConfirmation: true,
      baseOptions: [],
    }));
    useSessionStore.setState({ bridge, providers: await bridge.listProviders() });

    await useSessionStore.getState().startSession();

    expect(useSessionStore.getState().worktreeTransition).toBeNull();
    expect(vi.mocked(bridge.createManagedWorktree)).not.toHaveBeenCalled();
    expect(vi.mocked(bridge.openSession).mock.calls[0]?.[2]).toMatchObject({
      kind: "new",
      options: { cwd: sourceRoot },
    });
  });

  it("cancels a requested branch change without starting a chat", async () => {
    const bridge = bridgeFor(plan({
      action: "preserve_changes",
      preservation: "changes_remain_in_source",
      requiresConfirmation: true,
      targetBranch: "feature/target",
    }));
    useSessionStore.setState({
      bridge,
      providers: await bridge.listProviders(),
      worktreeTransition: await bridge.planProjectWorktree!(sourceRoot, "feature/target"),
    });

    useSessionStore.getState().dismissManagedWorktreeStart();

    expect(useSessionStore.getState().worktreeTransition).toBeNull();
    expect(vi.mocked(bridge.openSession)).not.toHaveBeenCalled();
    expect(useSessionStore.getState().notice).toContain("Branch change cancelled");
  });

  it("reuses a selected managed checkout instead of nesting another one", async () => {
    const bridge = bridgeFor(plan({
      sourceRoot: managedRoot,
      sourceIsManaged: true,
      sourceChanges: { changedFiles: 1, untrackedFiles: 0, conflictedFiles: 0 },
      preservation: "changes_remain_in_source",
      requiresConfirmation: true,
    }));
    useSessionStore.setState({
      bridge,
      providers: await bridge.listProviders(),
      localSettings: { ...baseSettings, cwd: managedRoot },
    });

    await useSessionStore.getState().startSession();

    expect(vi.mocked(bridge.createManagedWorktree)).not.toHaveBeenCalled();
    expect(vi.mocked(bridge.openSession).mock.calls[0]?.[2]).toMatchObject({
      kind: "new",
      options: { cwd: managedRoot },
    });
    expect(useSessionStore.getState().worktreeTransition).toBeNull();
  });

  it("preserves a dirty source while starting a requested-branch continuation", async () => {
    const bridge = bridgeFor(plan({
      action: "preserve_changes",
      preservation: "changes_remain_in_source",
      requiresConfirmation: true,
      targetBranch: "feature/target",
    }));
    useSessionStore.setState({
      bridge,
      providers: await bridge.listProviders(),
      worktreeTransition: await bridge.planProjectWorktree!(sourceRoot, "feature/target"),
    });

    await useSessionStore.getState().confirmManagedWorktreeStart();

    expect(vi.mocked(bridge.createManagedWorktree)).toHaveBeenCalledWith(sourceRoot, {
      base: "current",
      targetBranch: "feature/target",
    });
    expect(useSessionStore.getState().notice).toContain(
      "Started an isolated continuation from feature/target",
    );
    expect(useSessionStore.getState().notice).not.toContain("Current checkout");
    expect(useSessionStore.getState().localSettings.cwd).toBe(sourceRoot);
    expect(useSessionStore.getState().activeProjectRoot).toBe(managedRoot);
  });

  it("does not attach a worktree created for a project the user already left", async () => {
    let resolveCreate!: (value: Awaited<ReturnType<NonNullable<CoreBridge["createManagedWorktree"]>>>) => void;
    const bridge = bridgeFor(plan({
      sourceChanges: { changedFiles: 1, untrackedFiles: 0, conflictedFiles: 0 },
      preservation: "changes_remain_in_source",
      requiresConfirmation: true,
    }));
    vi.mocked(bridge.createManagedWorktree!).mockImplementation(
      () => new Promise((resolve) => { resolveCreate = resolve; }),
    );
    useSessionStore.setState({
      bridge,
      providers: await bridge.listProviders(),
      worktreeTransition: await bridge.planProjectWorktree!(sourceRoot),
    });

    const confirming = useSessionStore.getState().confirmManagedWorktreeStart();
    useSessionStore.getState().setProjectFolder("/tmp/other-project");
    resolveCreate({
      id: "session-1",
      label: "session",
      path: managedRoot,
      sourceRoot,
      base: "current",
      baseReference: "main",
      baseRevision: "0123456789abcdef0123456789abcdef01234567",
      createdAtMs: 1,
      state: "ready",
      changes: { changedFiles: 0, untrackedFiles: 0, conflictedFiles: 0 },
    });
    await confirming;

    expect(vi.mocked(bridge.cleanupManagedWorktree)).toHaveBeenCalledWith(sourceRoot, "session-1");
    expect(vi.mocked(bridge.openSession)).not.toHaveBeenCalled();
    expect(useSessionStore.getState().localSettings.cwd).toBe("/tmp/other-project");
    expect(useSessionStore.getState().pendingManagedWorktreePath).toBeNull();
  });

  it("starts immediately in the dirty checkout with a submitted draft", async () => {
    const bridge = bridgeFor(plan({
      sourceChanges: { changedFiles: 2, untrackedFiles: 1, conflictedFiles: 0 },
      preservation: "changes_remain_in_source",
      requiresConfirmation: true,
    }));
    useSessionStore.setState({ bridge, providers: await bridge.listProviders() });
    saveComposerDraft(draftOwner, null, "fix the login bug");

    await useSessionStore.getState().startSession({ submittedDraft: "fix the login bug" });

    expect(useSessionStore.getState().worktreeTransition).toBeNull();
    expect(useSessionStore.getState().session?.id).toBe("managed-chat");
    expect(vi.mocked(bridge.createManagedWorktree)).not.toHaveBeenCalled();
    expect(vi.mocked(bridge.openSession).mock.calls[0]?.[2]).toMatchObject({
      kind: "new",
      options: { cwd: sourceRoot },
    });
  });

  it("keeps the New session draft untouched on a dirty direct start", async () => {
    const bridge = bridgeFor(plan({
      sourceChanges: { changedFiles: 1, untrackedFiles: 1, conflictedFiles: 0 },
      preservation: "changes_remain_in_source",
      requiresConfirmation: true,
    }));
    useSessionStore.setState({ bridge, providers: await bridge.listProviders() });
    saveComposerDraft(draftOwner, null, "refactor the checkout flow");

    await useSessionStore.getState().startSession({ submittedDraft: "refactor the checkout flow" });

    const startedId = useSessionStore.getState().session?.id;
    expect(startedId).toBe("managed-chat");
    expect(loadComposerDraft(draftOwner, startedId!)).toBe("");
    expect(loadComposerDraft(draftOwner, null)).toBe("refactor the checkout flow");
  });

  it("keeps an unrelated New session draft out of a normally created conversation", async () => {
    const bridge = bridgeFor(plan());
    useSessionStore.setState({ bridge, providers: await bridge.listProviders() });
    saveComposerDraft(draftOwner, null, "UNSENT REGULAR SESSION ONLY");

    await useSessionStore.getState().startSession({ submittedDraft: "accepted developer prompt" });

    const startedId = useSessionStore.getState().session?.id;
    expect(startedId).toBe("managed-chat");
    expect(loadComposerDraft(draftOwner, startedId!)).toBe("");
    expect(loadComposerDraft(draftOwner, null)).toBe("UNSENT REGULAR SESSION ONLY");
  });
});
