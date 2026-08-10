import { useEffect, useMemo, useRef, useState } from "react";
import {
  Check,
  ChevronDown,
  FolderOpen,
  GitBranch,
  GitFork,
  Loader2,
  Search,
} from "lucide-react";
import type {
  ProjectBranch,
  ProjectContext,
  ProjectWorktreeTransitionPlan,
  RemoteWorkerTarget,
} from "../core-bridge/bridge";
import { getBridge } from "../core-bridge/bridge";
import { resolveBranchSelection } from "../lib/projectBranches";

const ITEM =
  "flex h-[22px] min-w-0 items-center gap-1 rounded-md bg-composer-context px-1.5 text-xs font-medium leading-none";

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

function normalizedPath(path: string): string {
  return path.replaceAll("\\", "/").replace(/\/+$/, "");
}

function checkoutName(path: string): string {
  return normalizedPath(path).split("/").at(-1) || path;
}

export function canOfferPreservingWorktree(
  allowPreserveChanges: boolean,
  remote: RemoteWorkerTarget | null,
): boolean {
  return allowPreserveChanges && !remote;
}

/** Branch selection is intentionally available only before a conversation is
 * started. The native boundary repeats the clean-tree check immediately before
 * `git switch`, so stale UI activity cannot carry edits across branches. */
export function BranchPicker({
  cwd,
  context,
  remote = null,
  disabledReason,
  allowPreserveChanges = false,
  onSwitched,
  onOpenCheckout,
  onTransitionPlan,
}: {
  cwd: string;
  context: ProjectContext;
  remote?: RemoteWorkerTarget | null;
  disabledReason?: string;
  /** Dirty source changes may be preserved in place while a new detached
   * continuation starts from the requested branch. Active agents still block. */
  allowPreserveChanges?: boolean;
  onSwitched: () => void;
  onOpenCheckout: (path: string) => void;
  onTransitionPlan?: (plan: ProjectWorktreeTransitionPlan) => void;
}) {
  const [open, setOpen] = useState(false);
  const [branches, setBranches] = useState<ProjectBranch[]>([]);
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(false);
  const [switching, setSwitching] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const Icon = context.isWorktree ? GitFork : GitBranch;
  const tone = context.isWorktree ? "text-checkout-worktree" : "text-checkout-branch";
  const currentLabel = context.detached ? `detached@${context.branch}` : context.branch;
  const filtered = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    if (!needle) return branches;
    return branches.filter((branch) => branch.name.toLocaleLowerCase().includes(needle));
  }, [branches, query]);
  const canPreserveChanges = canOfferPreservingWorktree(allowPreserveChanges, remote);

  useEffect(() => {
    if (!open) return;
    const closeOutside = (event: MouseEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) setOpen(false);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", closeOutside);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", closeOutside);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [open]);

  const showBranches = async () => {
    if (open) {
      setOpen(false);
      return;
    }
    setOpen(true);
    setQuery("");
    setError(null);
    setLoading(true);
    try {
      const bridge = await getBridge();
      if (!bridge.listProjectBranches) {
        throw new Error("Branch switching is available in the desktop app.");
      }
      setBranches(await bridge.listProjectBranches(cwd, remote));
    } catch (cause) {
      setBranches([]);
      setError(errorMessage(cause));
    } finally {
      setLoading(false);
    }
  };

  const chooseBranch = async (branch: ProjectBranch) => {
    const selection = resolveBranchSelection(branch, {
      cwd,
      branch: context.branch,
      detached: context.detached,
    });
    if (selection.action === "current") {
      setOpen(false);
      return;
    }
    setSwitching(branch.name);
    setError(null);
    try {
      const bridge = await getBridge();
      const plan = !remote && bridge.planProjectWorktree
        ? await bridge.planProjectWorktree(cwd, branch.name)
        : null;
      if (plan?.action === "open_owner" && plan.targetCheckoutPath) {
        onOpenCheckout(plan.targetCheckoutPath);
        setOpen(false);
        return;
      }
      if (plan?.action === "preserve_changes") {
        if (!canPreserveChanges) {
          throw new Error(
            disabledReason
              ?? "Wait for the active agent before starting an isolated branch continuation.",
          );
        }
        if (!onTransitionPlan) {
          throw new Error("Preserving changes in an isolated continuation is available in the desktop app.");
        }
        onTransitionPlan(plan);
        setOpen(false);
        return;
      }
      if (selection.action === "open") {
        onOpenCheckout(selection.path);
        setOpen(false);
        return;
      }
      if (disabledReason) {
        setError(disabledReason);
        return;
      }
      if (plan && plan.action !== "switch_clean") {
        throw new Error("This branch transition needs a different checkout choice.");
      }
      if (!bridge.switchProjectBranch) {
        throw new Error("Branch switching is available in the desktop app.");
      }
      await bridge.switchProjectBranch(cwd, branch.name, remote);
      setOpen(false);
      onSwitched();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setSwitching(null);
    }
  };

  return (
    <div ref={ref} className="relative shrink-0">
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`Switch branch. Current branch: ${currentLabel}`}
        title={
          disabledReason
            ? `Browse branches · ${disabledReason}`
            : `Switch branch · current: ${currentLabel}`
        }
        onClick={() => void showBranches()}
        className={`${ITEM} ${tone} transition hover:bg-bg-hover`}
      >
        <Icon className="size-3 shrink-0" />
        <span className="max-w-48 truncate">{currentLabel}</span>
        <ChevronDown className="size-2.5 shrink-0 opacity-65" />
      </button>

      {open && (
        <div
          role="menu"
          aria-label="Switch branch"
          className="popover-surface absolute bottom-full left-0 z-50 mb-2 w-72 overflow-hidden rounded-2xl bg-bg-elevated shadow-lifted ring-1 ring-border-subtle"
        >
          <div className="border-b border-border-subtle p-2">
            <label className="flex h-8 items-center gap-2 rounded-lg bg-bg-sunken px-2.5 ring-1 ring-border-subtle focus-within:ring-border-strong">
              <Search className="size-3.5 shrink-0 text-ink-faint" />
              <input
                autoFocus
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder="Find a local branch"
                aria-label="Find a local branch"
                className="min-w-0 flex-1 bg-transparent text-sm text-ink outline-none placeholder:text-ink-faint"
              />
            </label>
          </div>

          <div className="max-h-64 overflow-y-auto p-1.5">
            {loading && (
              <div className="flex items-center justify-center gap-2 px-3 py-5 text-xs text-ink-muted">
                <Loader2 className="size-3.5 animate-spin" /> Loading branches…
              </div>
            )}
            {!loading && filtered.map((branch) => {
              const current = !context.detached && branch.name === context.branch;
              const active = switching === branch.name;
              const ownedElsewhere = Boolean(
                branch.checkoutPath
                && normalizedPath(branch.checkoutPath) !== normalizedPath(cwd),
              );
              const switchBlocked = Boolean(
                disabledReason
                && !current
                && !ownedElsewhere
                && !canPreserveChanges,
              );
              return (
                <button
                  key={branch.name}
                  type="button"
                  role="menuitemradio"
                  aria-checked={current}
                  disabled={switching !== null || switchBlocked}
                  onClick={() => void chooseBranch(branch)}
                  title={
                    ownedElsewhere
                      ? `Open ${branch.checkoutPath}`
                      : switchBlocked
                        ? disabledReason
                        : undefined
                  }
                  className="flex min-h-8 w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-sm text-ink-secondary transition hover:bg-bg-hover hover:text-ink disabled:opacity-55"
                >
                  <span className="min-w-0 flex-1 truncate">{branch.name}</span>
                  {active ? (
                    <Loader2 className="size-3.5 shrink-0 animate-spin text-accent" />
                  ) : current ? (
                    <span className="flex items-center gap-1 text-xs text-accent">
                      Current <Check className="size-3.5 shrink-0" />
                    </span>
                  ) : ownedElsewhere ? (
                    <span
                      className="flex min-w-0 items-center gap-1 text-xs text-ink-faint"
                      title={checkoutName(branch.checkoutPath ?? "")}
                    >
                      Open checkout
                      <FolderOpen className="size-3.5 shrink-0" />
                    </span>
                  ) : canPreserveChanges && disabledReason ? (
                    <span className="flex items-center gap-1 text-xs text-checkout-worktree">
                      New worktree <GitFork className="size-3.5 shrink-0" />
                    </span>
                  ) : (
                    <span className="text-xs text-ink-faint">Switch here</span>
                  )}
                </button>
              );
            })}
            {!loading && !error && filtered.length === 0 && (
              <p className="px-3 py-5 text-center text-xs text-ink-faint">No matching branches.</p>
            )}
          </div>

          {error && (
            <p className="border-t border-border-subtle px-3 py-2.5 text-xs leading-4 text-danger">
              {error}
            </p>
          )}
          {!error && (
            <p className="border-t border-border-subtle px-3 py-2 text-xs text-ink-faint">
              {disabledReason
                ? canPreserveChanges
                  ? "Choose another branch to open it in a new worktree. Your current files stay here."
                  : disabledReason + " Branches already open elsewhere can still be opened."
                : "Choose a branch to switch this checkout. Branches already open elsewhere open that checkout."}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
