import { useEffect, useMemo, useRef, useState } from "react";
import { Check, ChevronDown, GitBranch, GitFork, Loader2, Search } from "lucide-react";
import type { ProjectContext } from "../core-bridge/bridge";
import { getBridge } from "../core-bridge/bridge";

const ITEM =
  "flex h-[22px] min-w-0 items-center gap-1 rounded-md bg-composer-context px-1.5 text-[11px] font-medium leading-none";

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

/** Branch selection is intentionally available only before a conversation is
 * started. The native boundary repeats the clean-tree check immediately before
 * `git switch`, so stale UI activity cannot carry edits across branches. */
export function BranchPicker({
  cwd,
  context,
  disabledReason,
  onSwitched,
}: {
  cwd: string;
  context: ProjectContext;
  disabledReason?: string;
  onSwitched: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [branches, setBranches] = useState<string[]>([]);
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
    return branches.filter((branch) => branch.toLocaleLowerCase().includes(needle));
  }, [branches, query]);

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
    if (disabledReason) return;
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
      setBranches(await bridge.listProjectBranches(cwd));
    } catch (cause) {
      setBranches([]);
      setError(errorMessage(cause));
    } finally {
      setLoading(false);
    }
  };

  const chooseBranch = async (branch: string) => {
    if (!context.detached && branch === context.branch) {
      setOpen(false);
      return;
    }
    setSwitching(branch);
    setError(null);
    try {
      const bridge = await getBridge();
      if (!bridge.switchProjectBranch) {
        throw new Error("Branch switching is available in the desktop app.");
      }
      await bridge.switchProjectBranch(cwd, branch);
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
        disabled={Boolean(disabledReason)}
        title={disabledReason ?? `Switch branch · current: ${currentLabel}`}
        onClick={() => void showBranches()}
        className={`${ITEM} ${tone} transition hover:bg-bg-hover disabled:cursor-not-allowed disabled:opacity-55`}
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
              const current = !context.detached && branch === context.branch;
              const active = switching === branch;
              return (
                <button
                  key={branch}
                  type="button"
                  role="menuitemradio"
                  aria-checked={current}
                  disabled={switching !== null}
                  onClick={() => void chooseBranch(branch)}
                  className="flex min-h-8 w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-sm text-ink-secondary transition hover:bg-bg-hover hover:text-ink disabled:opacity-55"
                >
                  <span className="min-w-0 flex-1 truncate" title={branch}>{branch}</span>
                  {active ? (
                    <Loader2 className="size-3.5 shrink-0 animate-spin text-accent" />
                  ) : current ? (
                    <Check className="size-3.5 shrink-0 text-accent" />
                  ) : null}
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
            <p className="border-t border-border-subtle px-3 py-2 text-[11px] text-ink-faint">
              Existing local branches only. Your working tree must be clean.
            </p>
          )}
        </div>
      )}
    </div>
  );
}
