import type {
  ManagedWorktree,
  ManagedWorktreeBranchReceipt,
  ManagedWorktreeCleanupReceipt,
  ManagedWorktreeRequest,
  ProjectBranch,
  ProjectContext,
  ProjectWorktreeTransitionPlan,
  WorktreeChangeSummary,
  WorktreeTransitionAction,
} from "../core-bridge/bridge";

export type FakeGitScenario = "clean" | "modified" | "untracked" | "conflicted";
export type FakeManagedScenario = "ready" | "dirty" | "committed";

const REVISION = "0123456789abcdef0123456789abcdef01234567";
const PRIVATE_REVISION = "fedcba9876543210fedcba9876543210fedcba98";

export const FAKE_GIT_SIMULATION_STORAGE_KEY = "agent-desktop:fake-git-simulation";

export function fakeGitScenario(): FakeGitScenario {
  if (typeof localStorage === "undefined") return "clean";
  // Browser previews can opt into a dirty checkout from the address bar so a
  // journey test does not need to reach into WebView storage or mutate a real
  // repository. Native execution never uses this deterministic double.
  const query = typeof window !== "undefined"
    ? new URLSearchParams(window.location.search).get("fakeGit")
    : null;
  const value = query ?? localStorage.getItem(FAKE_GIT_SIMULATION_STORAGE_KEY);
  return value === "modified" || value === "untracked" || value === "conflicted"
    ? value
    : "clean";
}

export function fakeGitChanges(scenario: FakeGitScenario): WorktreeChangeSummary {
  switch (scenario) {
    case "modified":
      return { changedFiles: 1, untrackedFiles: 0, conflictedFiles: 0 };
    case "untracked":
      return { changedFiles: 0, untrackedFiles: 1, conflictedFiles: 0 };
    case "conflicted":
      return { changedFiles: 1, untrackedFiles: 0, conflictedFiles: 1 };
    default:
      return { changedFiles: 0, untrackedFiles: 0, conflictedFiles: 0 };
  }
}

function dirty(changes: WorktreeChangeSummary): boolean {
  return changes.changedFiles + changes.untrackedFiles + changes.conflictedFiles > 0;
}

export interface FakeGitTransitionInput {
  sourceRoot: string;
  sourceBranch: string | null;
  sourceIsManaged: boolean;
  sourceChanges: WorktreeChangeSummary;
  targetBranch: string | null;
  targetExists: boolean;
  targetCheckoutPath: string | null;
}

export interface FakeGitTransitionDecision {
  action: WorktreeTransitionAction;
  preservation: ProjectWorktreeTransitionPlan["preservation"];
  requiresConfirmation: boolean;
  targetCheckoutPath: string | null;
}

export type FakeGitTransitionResult =
  | { ok: true; decision: FakeGitTransitionDecision }
  | { ok: false; error: string };

/** Pure, non-throwing router used by exhaustive simulations. */
function tryDecideFakeGitTransition(input: FakeGitTransitionInput): FakeGitTransitionResult {
  const {
    sourceRoot,
    sourceBranch,
    sourceIsManaged,
    sourceChanges,
    targetBranch,
    targetExists,
    targetCheckoutPath,
  } = input;
  const sourceDirty = dirty(sourceChanges);
  if (!targetBranch) {
    return { ok: true, decision: {
      action: "create_isolated", preservation: sourceDirty ? "changes_remain_in_source" : "clean",
      requiresConfirmation: sourceDirty, targetCheckoutPath: null,
    }};
  }
  if (!targetExists) return { ok: false, error: `Local branch ${targetBranch} no longer exists.` };
  if (targetCheckoutPath && normalize(targetCheckoutPath) !== normalize(sourceRoot)) {
    return { ok: true, decision: {
      action: "open_owner",
      preservation: "owner_checkout",
      requiresConfirmation: false,
      targetCheckoutPath,
    }};
  }
  if (sourceIsManaged && sourceBranch !== targetBranch) {
    return { ok: false, error: "This the agent-managed checkout is pinned to its existing branch. Start a new isolated session instead of switching this worktree." };
  }
  if (sourceBranch === targetBranch) {
    return { ok: true, decision: {
      action: "create_isolated",
      preservation: sourceDirty ? "changes_remain_in_source" : "clean",
      requiresConfirmation: sourceDirty,
      targetCheckoutPath: null,
    }};
  }
  return { ok: true, decision: {
    action: sourceDirty ? "preserve_changes" : "switch_clean",
    preservation: sourceDirty ? "changes_remain_in_source" : "clean",
    requiresConfirmation: sourceDirty,
    targetCheckoutPath: null,
  }};
}

/** Throwing adapter used by the fake bridge, matching native command errors. */
function decideFakeGitTransition(input: FakeGitTransitionInput): FakeGitTransitionDecision {
  const result = tryDecideFakeGitTransition(input);
  if (!result.ok) throw new Error(result.error);
  return result.decision;
}

function normalize(path: string): string {
  return path.replaceAll("\\", "/").replace(/\/+$/, "");
}

interface FakeCheckout {
  path: string;
  sourceRoot: string;
  branch: string | null;
  detached: boolean;
  headRevision: string;
  changes: WorktreeChangeSummary;
  managedId?: string;
}

/**
 * Deterministic Git/worktree double for browser previews and journey tests.
 * It models the safety boundary, not Git's object database: every mutating
 * method rechecks ownership and cleanliness exactly like the native command.
 */
export class FakeGitRepository {
  readonly root: string;
  private readonly checkouts = new Map<string, FakeCheckout>();
  private readonly branches = new Set(["main", "feature/checkout-context", "fix/composer-layout"]);
  private readonly managed = new Map<string, ManagedWorktree>();
  private sequence = 0;

  constructor(root: string, scenario: FakeGitScenario = "clean") {
    this.root = normalize(root);
    this.checkouts.set(this.root, {
      path: this.root,
      sourceRoot: this.root,
      branch: "main",
      detached: false,
      headRevision: REVISION,
      changes: fakeGitChanges(scenario),
    });
  }

  setScenario(scenario: FakeGitScenario): void {
    const source = this.checkouts.get(this.root);
    if (source) source.changes = fakeGitChanges(scenario);
  }

  context(path: string): ProjectContext | null {
    const checkout = this.checkouts.get(normalize(path));
    if (!checkout) return null;
    return {
      branch: checkout.branch ?? `detached@${checkout.headRevision.slice(0, 12)}`,
      detached: checkout.detached,
      isWorktree: Boolean(checkout.managedId),
      worktreeRoot: checkout.path,
      activity: {
        ...checkout.changes,
        externalAgents: [],
        detectedAtMs: 1,
      },
    };
  }

  listBranches(path: string): ProjectBranch[] {
    const checkout = this.requireCheckout(path);
    return [...this.branches].sort().map((name) => ({
      name,
      checkoutPath: [...this.checkouts.values()].find((candidate) => candidate.branch === name)?.path ?? null,
    })).map((branch) => branch.name === checkout.branch
      ? { ...branch, checkoutPath: checkout.path }
      : branch);
  }

  plan(path: string, targetBranch?: string | null): ProjectWorktreeTransitionPlan {
    const checkout = this.requireCheckout(path);
    const target = targetBranch?.trim() || null;
    if (target && !this.branches.has(target)) {
      throw new Error(`Local branch ${target} no longer exists.`);
    }
    const owner = target
      ? this.listBranches(checkout.path).find((branch) => branch.name === target)?.checkoutPath ?? null
      : null;
    const decision = decideFakeGitTransition({
      sourceRoot: checkout.path,
      sourceBranch: checkout.branch,
      sourceIsManaged: Boolean(checkout.managedId),
      sourceChanges: checkout.changes,
      targetBranch: target,
      targetExists: true,
      targetCheckoutPath: owner,
    });
    return {
      sourceRoot: checkout.path,
      sourceBranch: checkout.branch,
      sourceRevision: checkout.headRevision,
      sourceChanges: checkout.changes,
      sourceIsManaged: Boolean(checkout.managedId),
      targetBranch: target,
      targetCheckoutPath: decision.targetCheckoutPath,
      action: decision.action,
      preservation: decision.preservation,
      requiresConfirmation: decision.requiresConfirmation,
      baseOptions: [
        {
          id: "current",
          label: `Current checkout (${checkout.branch ?? "HEAD"})`,
          reference: checkout.branch ?? "HEAD",
          revision: checkout.headRevision,
          fallback: false,
        },
        {
          id: "default",
          label: "Default branch (main)",
          reference: "main",
          revision: REVISION,
          fallback: false,
        },
      ],
      managedLocation: `${checkout.sourceRoot}.agent-worktrees`,
    };
  }

  switchBranch(path: string, branch: string): void {
    const checkout = this.requireCheckout(path);
    if (!this.branches.has(branch)) throw new Error(`Local branch ${branch} no longer exists.`);
    if (checkout.managedId) throw new Error("This the agent-managed checkout is pinned to its existing branch. Start a new isolated session instead of switching this worktree.");
    if (checkout.branch === branch) return;
    if (dirty(checkout.changes)) throw new Error("Commit or remove local changes before switching branches.");
    const owner = [...this.checkouts.values()].find((candidate) => candidate.branch === branch);
    if (owner && owner.path !== checkout.path) throw new Error(`Branch ${branch} is already checked out at ${owner.path}. Open that checkout instead.`);
    checkout.branch = branch;
    checkout.detached = false;
  }

  createManaged(path: string, request: ManagedWorktreeRequest, scenario: FakeManagedScenario = "ready"): ManagedWorktree {
    const source = this.requireCheckout(path);
    if (source.managedId) throw new Error("This checkout is already a the agent-managed isolated worktree. Reuse it instead of nesting another checkout.");
    const id = `session-${++this.sequence}`;
    const managedPath = `${source.sourceRoot}.agent-worktrees/${id}`;
    const branch = `agent/${id}`;
    this.branches.add(branch);
    const headRevision = scenario === "committed" ? PRIVATE_REVISION : REVISION;
    const checkout: FakeCheckout = {
      path: managedPath,
      sourceRoot: source.sourceRoot,
      branch,
      detached: false,
      headRevision,
      changes: scenario === "dirty" ? { changedFiles: 1, untrackedFiles: 0, conflictedFiles: 0 } : fakeGitChanges("clean"),
      managedId: id,
    };
    this.checkouts.set(managedPath, checkout);
    const created: ManagedWorktree = {
      id,
      label: request.label?.trim() || "session",
      path: managedPath,
      sourceRoot: source.sourceRoot,
      base: request.base,
      baseReference: request.targetBranch?.trim() || (request.base === "default" ? "main" : source.branch ?? "HEAD"),
      baseRevision: REVISION,
      headRevision,
      preservedBranch: branch,
      createdAtMs: this.sequence,
      state: scenario,
      changes: checkout.changes,
    };
    this.managed.set(id, created);
    return created;
  }

  listManaged(path: string): ManagedWorktree[] {
    const sourceRoot = this.requireCheckout(path).sourceRoot;
    return [...this.managed.values()].filter((worktree) => worktree.sourceRoot === sourceRoot);
  }

  cleanupManaged(path: string, id: string): ManagedWorktreeCleanupReceipt {
    const sourceRoot = this.requireCheckout(path).sourceRoot;
    const worktree = this.managed.get(id);
    if (!worktree || worktree.sourceRoot !== sourceRoot) throw new Error("That managed worktree is not registered for this repository.");
    if (worktree.state === "dirty") throw new Error("Managed worktree has local changes. Commit, move, or remove them before cleanup.");
    if (worktree.state === "committed") throw new Error("Managed worktree has commits that are not protected by a branch.");
    this.managed.delete(id);
    this.checkouts.delete(normalize(worktree.path));
    return { id, path: worktree.path, removed: true };
  }

  saveManaged(path: string, id: string): ManagedWorktreeBranchReceipt {
    const sourceRoot = this.requireCheckout(path).sourceRoot;
    const worktree = this.managed.get(id);
    if (!worktree || worktree.sourceRoot !== sourceRoot) throw new Error("That managed worktree is not registered for this repository.");
    if (worktree.state === "dirty") throw new Error("Managed worktree still has local changes.");
    if (worktree.state !== "committed" && worktree.state !== "saved") throw new Error("This managed worktree has no new commits to save as a branch.");
    if (worktree.state === "saved") {
      return {
        id,
        path: worktree.path,
        branch: worktree.preservedBranch ?? `agent/${id}`,
        headRevision: worktree.headRevision ?? REVISION,
      };
    }
    const branch = `${worktree.preservedBranch ?? `agent/${id}`}-saved`;
    this.branches.add(branch);
    worktree.preservedBranch = branch;
    worktree.state = "saved";
    return { id, path: worktree.path, branch, headRevision: worktree.headRevision ?? REVISION };
  }

  private requireCheckout(path: string): FakeCheckout {
    const checkout = this.checkouts.get(normalize(path));
    if (!checkout) throw new Error("Project folder is not a known fake Git checkout.");
    return checkout;
  }
}
