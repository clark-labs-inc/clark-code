import { useCallback, useEffect, useRef, useState } from "react";
import { Archive, Check, FolderOpen, GitBranch, GitFork, Loader2 } from "lucide-react";
import type { ManagedWorktree } from "../core-bridge/bridge";

function sameCheckout(left: string, right: string): boolean {
  return left.replaceAll("\\", "/").replace(/\/+$/, "")
    === right.replaceAll("\\", "/").replace(/\/+$/, "");
}

function changeSummary(worktree: ManagedWorktree): string {
  const parts: string[] = [];
  if (worktree.changes.changedFiles) parts.push(`${worktree.changes.changedFiles} changed`);
  if (worktree.changes.untrackedFiles) parts.push(`${worktree.changes.untrackedFiles} untracked`);
  if (worktree.changes.conflictedFiles) parts.push(`${worktree.changes.conflictedFiles} conflicted`);
  return parts.join(", ");
}

function lifecycleCopy(worktree: ManagedWorktree, inUse: boolean): string {
  if (worktree.state === "missing") {
    return "This checkout is missing. Clark leaves its registry entry untouched.";
  }
  if (inUse) {
    return "A live Clark chat still uses this checkout. Close or archive that chat before archiving it.";
  }
  if (worktree.state === "dirty") {
    return `${changeSummary(worktree)}. Commit, move, or remove those changes before continuing.`;
  }
  if (worktree.state === "committed") {
    return "New commits are not protected by a branch. Save them before archiving this checkout.";
  }
  if (worktree.state === "saved") {
    return `Commits are saved as ${worktree.preservedBranch || "a local branch"}. This checkout can now be archived.`;
  }
  return "No local edits or private commits. This checkout is ready to archive.";
}

export function ManagedWorktreeManager({
  loadWorktrees,
  onUseWorktree,
  onSaveBranch,
  onArchiveCheckout,
  activeWorktreePaths,
  onBack,
}: {
  loadWorktrees: () => Promise<ManagedWorktree[]>;
  onUseWorktree: (path: string) => void;
  onSaveBranch: (id: string) => Promise<{ branch: string }>;
  onArchiveCheckout: (id: string) => Promise<void>;
  /** Every live session, including an idle background chat, keeps its checkout. */
  activeWorktreePaths: string[];
  onBack: () => void;
}) {
  const loader = useRef(loadWorktrees);
  const [worktrees, setWorktrees] = useState<ManagedWorktree[]>([]);
  const [loading, setLoading] = useState(true);
  const [savingId, setSavingId] = useState<string | null>(null);
  const [archivingId, setArchivingId] = useState<string | null>(null);
  const [confirmingId, setConfirmingId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    loader.current = loadWorktrees;
  }, [loadWorktrees]);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setWorktrees(await loader.current());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const saveBranch = async (id: string) => {
    if (savingId || archivingId) return;
    setSavingId(id);
    setError(null);
    try {
      const receipt = await onSaveBranch(id);
      setWorktrees((current) => current.map((worktree) => (
        worktree.id === id
          ? { ...worktree, state: "saved", preservedBranch: receipt.branch }
          : worktree
      )));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSavingId(null);
    }
  };

  const archive = async (id: string) => {
    if (savingId || archivingId) return;
    setArchivingId(id);
    setError(null);
    try {
      await onArchiveCheckout(id);
      setWorktrees((current) => current.filter((worktree) => worktree.id !== id));
      setConfirmingId(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setArchivingId(null);
    }
  };

  const busy = savingId !== null || archivingId !== null;

  return (
    <div className="p-1.5">
      <div className="mb-2 flex items-center justify-between gap-2 px-1">
        <div>
          <span className="block text-xs font-medium text-ink-muted">Isolated worktrees</span>
          <span className="block text-[10px] text-ink-faint">Protect work, then archive the checkout.</span>
        </div>
        <button
          type="button"
          disabled={loading || busy}
          onClick={() => void refresh()}
          className="rounded-md px-1.5 py-1 text-[11px] text-ink-muted transition hover:bg-bg-hover hover:text-ink disabled:opacity-40"
        >
          Refresh
        </button>
      </div>

      {loading ? (
        <div className="flex items-center justify-center gap-2 px-3 py-6 text-xs text-ink-muted">
          <Loader2 className="size-3.5 animate-spin" /> Checking worktrees…
        </div>
      ) : worktrees.length === 0 ? (
        <p className="px-3 py-6 text-center text-xs leading-4 text-ink-faint">
          No isolated worktrees need attention for this project.
        </p>
      ) : (
        <div className="max-h-72 space-y-1.5 overflow-y-auto pr-0.5">
          {worktrees.map((worktree) => {
            const inUse = activeWorktreePaths.some((path) => sameCheckout(path, worktree.path));
            const canUse = worktree.state !== "missing" && !busy;
            const canSave = worktree.state === "committed" && !busy;
            const canArchive = (worktree.state === "ready" || worktree.state === "saved") && !inUse && !busy;
            const saving = savingId === worktree.id;
            const archiving = archivingId === worktree.id;
            const confirming = confirmingId === worktree.id;
            return (
              <article key={worktree.id} className="rounded-xl bg-bg-sunken p-2.5 ring-1 ring-border-subtle">
                <div className="flex items-center gap-2">
                  <span className={"grid size-5 shrink-0 place-items-center rounded-md " + (
                    worktree.state === "saved" ? "bg-success/10 text-success" : "bg-checkout-worktree/10 text-checkout-worktree"
                  )}>
                    {worktree.state === "saved" ? <Check className="size-3" /> : <GitFork className="size-3" />}
                  </span>
                  <span className="min-w-0 flex-1 truncate text-xs font-medium text-ink-secondary">{worktree.label}</span>
                  <span className="text-[10px] font-medium uppercase tracking-wide text-ink-faint">
                    {inUse ? "in use" : worktree.state}
                  </span>
                </div>
                <p className="mt-1 truncate font-mono text-[10px] text-ink-faint" title={worktree.path}>{worktree.path}</p>
                <p className="mt-2 text-[11px] leading-4 text-ink-muted">{lifecycleCopy(worktree, inUse)}</p>

                {confirming ? (
                  <div className="mt-2 rounded-lg border border-danger/20 bg-danger/5 p-2">
                    <p className="text-[11px] leading-4 text-ink-secondary">
                      Archive this checkout? Its chat history stays available{worktree.state === "saved" ? ", and the saved branch stays in Git" : ""}.
                    </p>
                    <div className="mt-2 flex justify-end gap-1.5">
                      <button
                        type="button"
                        disabled={archiving}
                        onClick={() => setConfirmingId(null)}
                        className="rounded-md px-2 py-1 text-[11px] text-ink-muted hover:bg-bg-hover hover:text-ink disabled:opacity-40"
                      >
                        Keep checkout
                      </button>
                      <button
                        type="button"
                        disabled={archiving}
                        onClick={() => void archive(worktree.id)}
                        className="flex items-center gap-1 rounded-md bg-danger/10 px-2 py-1 text-[11px] font-medium text-danger hover:bg-danger/20 disabled:opacity-40"
                      >
                        {archiving && <Loader2 className="size-3 animate-spin" />}
                        Archive checkout
                      </button>
                    </div>
                  </div>
                ) : (
                  <div className="mt-2 flex flex-wrap justify-end gap-1">
                    <button
                      type="button"
                      disabled={!canUse}
                      title={canUse ? "Use this isolated checkout for a new chat" : "This checkout is no longer available"}
                      onClick={() => onUseWorktree(worktree.path)}
                      className="flex items-center gap-1 rounded-md px-1.5 py-1 text-[11px] text-ink-muted transition hover:bg-bg-hover hover:text-ink disabled:cursor-not-allowed disabled:opacity-40"
                    >
                      <FolderOpen className="size-3" />
                      Use for new chat
                    </button>
                    {worktree.state === "committed" && (
                      <button
                        type="button"
                        disabled={!canSave}
                        onClick={() => void saveBranch(worktree.id)}
                        className="flex items-center gap-1 rounded-md bg-accent-subtle px-1.5 py-1 text-[11px] font-medium text-accent transition hover:bg-accent/15 disabled:opacity-40"
                      >
                        {saving ? <Loader2 className="size-3 animate-spin" /> : <GitBranch className="size-3" />}
                        Save commits as branch
                      </button>
                    )}
                    {(worktree.state === "ready" || worktree.state === "saved") && (
                      <button
                        type="button"
                        disabled={!canArchive}
                        title={
                          inUse
                            ? "A live Clark session is still using this checkout"
                            : canArchive
                              ? "Archive this empty checkout"
                              : "This checkout cannot be archived yet"
                        }
                        onClick={() => setConfirmingId(worktree.id)}
                        className="flex items-center gap-1 rounded-md px-1.5 py-1 text-[11px] text-danger transition hover:bg-danger/10 disabled:cursor-not-allowed disabled:opacity-40"
                      >
                        <Archive className="size-3" />
                        Archive checkout
                      </button>
                    )}
                  </div>
                )}
              </article>
            );
          })}
        </div>
      )}

      {error && <p className="mt-2 px-1 text-xs leading-4 text-danger">{error}</p>}
      <div className="mt-2 flex justify-end">
        <button
          type="button"
          disabled={busy}
          onClick={onBack}
          className="rounded-lg px-2.5 py-1.5 text-xs text-ink-muted hover:bg-bg-hover hover:text-ink disabled:opacity-40"
        >
          Back
        </button>
      </div>
    </div>
  );
}
