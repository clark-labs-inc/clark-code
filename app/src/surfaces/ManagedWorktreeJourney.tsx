import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown, GitFork, Loader2 } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";

const CHIP =
  "flex h-[22px] min-w-0 items-center gap-1 rounded-md bg-composer-context px-1.5 text-[11px] font-medium leading-none text-checkout-worktree transition hover:bg-bg-hover";

function changeSummary(changes: {
  changedFiles: number;
  untrackedFiles: number;
  conflictedFiles: number;
}): string {
  const parts: string[] = [];
  if (changes.changedFiles) parts.push(changes.changedFiles + " changed");
  if (changes.untrackedFiles) parts.push(changes.untrackedFiles + " untracked");
  if (changes.conflictedFiles) parts.push(changes.conflictedFiles + " conflicted");
  return parts.join(", ") || "no local changes";
}

/** Base selector for the next isolated session. The host resolves the exact
 * revision on launch, so this remains an intent picker rather than a stale ref
 * cache in the renderer. */
export function ManagedWorktreeBasePicker() {
  const base = useSessionStore((state) => state.managedWorktreeBase);
  const setBase = useSessionStore((state) => state.setManagedWorktreeBase);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const label = base === "default" ? "fresh default branch" : "current checkout";

  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", escape);
    };
  }, [open]);

  return (
    <div ref={ref} className="relative shrink-0">
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={"New isolated session base: " + label}
        title={"New isolated session from " + label}
        onClick={() => setOpen((current) => !current)}
        className={CHIP}
      >
        <GitFork className="size-3 shrink-0" />
        <span className="max-w-32 truncate">Isolated · {base === "default" ? "Fresh default" : "Current"}</span>
        <ChevronDown className="size-2.5 shrink-0 opacity-65" />
      </button>
      {open && (
        <div
          role="menu"
          aria-label="New isolated session base"
          className="popover-surface absolute bottom-full left-0 z-50 mb-2 w-72 rounded-xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle"
        >
          <p className="px-2.5 pb-1.5 pt-1 text-[11px] leading-4 text-ink-faint">
            New chats receive their own branch-backed checkout. Existing chats remain pinned.
          </p>
          {([
            ["current", "Current checkout", "Exact current commit; never fetches or changes this checkout."],
            ["default", "Fresh default branch", "Refreshes origin's default just before starting; falls back safely if it is unavailable."],
          ] as const).map(([id, title, description]) => (
            <button
              key={id}
              type="button"
              role="menuitemradio"
              aria-checked={base === id}
              onClick={() => {
                setBase(id);
                setOpen(false);
              }}
              className="flex w-full items-start gap-2 rounded-lg px-2.5 py-2 text-left transition hover:bg-bg-hover"
            >
              <span className="mt-0.5 grid size-3.5 shrink-0 place-items-center">
                {base === id && <Check className="size-3.5 text-accent" />}
              </span>
              <span className="min-w-0">
                <span className="block text-xs font-medium text-ink-secondary">{title}</span>
                <span className="mt-0.5 block text-[11px] leading-4 text-ink-faint">{description}</span>
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** The source checkout is dirty. This is the deliberate preservation gate: the
 * user chooses an immutable base, then Clark creates a clean sibling checkout
 * while leaving every source change exactly where it was. */
export function ManagedWorktreeTransitionDialog() {
  const plan = useSessionStore((state) => state.worktreeTransition);
  const base = useSessionStore((state) => state.managedWorktreeBase);
  const setBase = useSessionStore((state) => state.setManagedWorktreeBase);
  const confirm = useSessionStore((state) => state.confirmManagedWorktreeStart);
  const dismiss = useSessionStore((state) => state.dismissManagedWorktreeStart);
  const preparing = useSessionStore((state) => state.worktreePreparing);
  if (!plan) return null;

  const selected = plan.baseOptions.find((option) => option.id === base) ?? plan.baseOptions[0];
  const changes = changeSummary(plan.sourceChanges);
  const targetBranch = plan.action === "preserve_changes" ? plan.targetBranch : null;

  return (
    <div
      className="fixed inset-0 z-[70] flex items-center justify-center bg-black/35 p-4 backdrop-blur-[1px]"
      role="presentation"
    >
      <section
        role="dialog"
        aria-modal="true"
        aria-labelledby="managed-worktree-title"
        className="w-full max-w-md rounded-2xl bg-bg-elevated p-5 shadow-lifted ring-1 ring-border-subtle"
      >
        <div className="flex items-start gap-3">
          <span className="grid size-9 shrink-0 place-items-center rounded-xl bg-checkout-worktree/10 text-checkout-worktree">
            <GitFork className="size-4" />
          </span>
          <div className="min-w-0">
            <h2 id="managed-worktree-title" className="text-base font-semibold text-ink">
              Keep this work safe. Start an isolated {targetBranch ? "continuation" : "chat"}
            </h2>
            <p className="mt-1 text-sm leading-5 text-ink-muted">
              This checkout has {changes}. Clark will not stash, commit, discard, or carry those changes across a branch.
            </p>
          </div>
        </div>

        <div className="mt-4 rounded-xl bg-bg-sunken p-3 text-xs leading-5 text-ink-muted">
          <span className="font-medium text-ink-secondary">What stays here:</span>
          <span className="ml-1 break-all">{plan.sourceRoot}</span>
        </div>

        {targetBranch ? (
          <div className="mt-4 rounded-xl border border-border-subtle p-3">
            <span className="block text-xs font-semibold uppercase tracking-[0.1em] text-ink-muted">
              Isolated continuation branch
            </span>
            <span className="mt-1 block break-all font-mono text-sm text-ink">{targetBranch}</span>
            <span className="mt-1 block text-xs leading-4 text-ink-faint">
              The source changes remain where they are; the new isolated checkout starts from this branch.
            </span>
          </div>
        ) : (
        <fieldset className="mt-4">
          <legend className="mb-2 text-xs font-semibold uppercase tracking-[0.1em] text-ink-muted">
            Isolated checkout base
          </legend>
          <div className="space-y-2">
            {plan.baseOptions.map((option) => (
              <button
                key={option.id}
                type="button"
                disabled={preparing}
                onClick={() => setBase(option.id)}
                className={"flex w-full items-start gap-2.5 rounded-xl border p-3 text-left transition disabled:opacity-50 " + (
                  base === option.id
                    ? "border-accent bg-accent-subtle"
                    : "border-border-subtle hover:border-border"
                )}
              >
                <span
                  className={"mt-0.5 grid size-4 shrink-0 place-items-center rounded-full border " + (
                    base === option.id ? "border-accent bg-accent text-white" : "border-ink-faint"
                  )}
                >
                  {base === option.id && <Check className="size-3" />}
                </span>
                <span className="min-w-0">
                  <span className="block text-sm font-medium text-ink">{option.label}</span>
                  <span className="mt-0.5 block break-all font-mono text-[11px] text-ink-faint">
                    {option.reference} at {option.revision.slice(0, 12)}
                  </span>
                  {option.fallback && (
                    <span className="mt-1 block text-[11px] leading-4 text-ink-muted">
                      No default branch was found locally, so this uses the current commit.
                    </span>
                  )}
                </span>
              </button>
            ))}
          </div>
        </fieldset>
        )}

        <p className="mt-3 text-xs leading-4 text-ink-faint">
          The new checkout has its own branch. Clark can archive it only after local changes are resolved and live chats are closed; legacy detached work must be saved as a branch first.
        </p>

        <div className="mt-5 flex justify-end gap-2">
          <button
            type="button"
            disabled={preparing}
            onClick={dismiss}
            className="rounded-lg px-3 py-2 text-sm text-ink-muted transition hover:bg-bg-hover hover:text-ink disabled:opacity-50"
          >
            Keep working here
          </button>
          <button
            type="button"
            disabled={preparing || !selected}
            onClick={() => void confirm()}
            className="flex items-center gap-2 rounded-lg bg-accent px-3 py-2 text-sm font-medium text-white transition hover:bg-accent-hover disabled:opacity-50"
          >
            {preparing && <Loader2 className="size-3.5 animate-spin" />}
            Start isolated {targetBranch ? "continuation" : "chat"}
          </button>
        </div>
      </section>
    </div>
  );
}
