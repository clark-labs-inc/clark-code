import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import {
  ArrowRight,
  Check,
  ChevronDown,
  GitBranch,
  GitFork,
  Loader2,
} from "lucide-react";
import type { ManagedWorktreeBase, ProjectWorktreeTransitionPlan } from "../core-bridge/bridge";
import { useSessionStore } from "../store/sessionStore";
import { useModalFocus } from "../lib/modalFocus";

const CHIP =
  "flex h-[22px] min-w-0 items-center gap-1 rounded-md bg-composer-context px-1.5 text-xs font-medium leading-none text-checkout-worktree transition hover:bg-bg-hover";

function changeSummary(changes: {
  changedFiles: number;
  untrackedFiles: number;
  conflictedFiles: number;
}): string {
  const parts: string[] = [];
  if (changes.changedFiles) {
    parts.push(`${changes.changedFiles} changed file${changes.changedFiles === 1 ? "" : "s"}`);
  }
  if (changes.untrackedFiles) {
    parts.push(`${changes.untrackedFiles} untracked file${changes.untrackedFiles === 1 ? "" : "s"}`);
  }
  if (changes.conflictedFiles) {
    parts.push(`${changes.conflictedFiles} conflicted file${changes.conflictedFiles === 1 ? "" : "s"}`);
  }
  return parts.join(", ") || "no local changes";
}

/** Starting-point selector for the next session. The host resolves the exact
 * revision on launch, so this remains an intent picker rather than a stale ref
 * cache in the renderer. */
export function ManagedWorktreeBasePicker() {
  const base = useSessionStore((state) => state.managedWorktreeBase);
  const setBase = useSessionStore((state) => state.setManagedWorktreeBase);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const label = base === "default" ? "default branch" : "this checkout";
  const Icon = base === "default" ? GitFork : GitBranch;

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
        aria-label={"New chat starts in " + label}
        title={"New chat starts in " + label}
        onClick={() => setOpen((current) => !current)}
        className={CHIP}
      >
        <Icon className="size-3 shrink-0" />
        <span className="max-w-36 truncate">
          New chat · {base === "default" ? "Default branch" : "This checkout"}
        </span>
        <ChevronDown className="size-2.5 shrink-0 opacity-65" />
      </button>
      {open && (
        <div
          role="menu"
          aria-label="New chat starting point"
          className="popover-surface absolute bottom-full left-0 z-50 mb-2 w-72 rounded-xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle"
        >
          <p className="px-2.5 pb-1.5 pt-1 text-xs leading-4 text-ink-faint">
            Choose where new chats begin. This never moves your uncommitted files.
          </p>
          {([
            ["current", "This checkout", "Start directly in the selected checkout; no worktree is created."],
            ["default", "Default branch", "Create an isolated worktree from the latest available default branch."],
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
                <span className="mt-0.5 block text-xs leading-4 text-ink-faint">{description}</span>
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** The source checkout is dirty. This is the deliberate preservation gate: the
 * user chooses an immutable base, then the agent creates a clean sibling checkout
 * while leaving every source change exactly where it was. */
export function ManagedWorktreeTransitionDialog() {
  const plan = useSessionStore((state) => state.worktreeTransition);
  const base = useSessionStore((state) => state.managedWorktreeBase);
  const setBase = useSessionStore((state) => state.setManagedWorktreeBase);
  const confirm = useSessionStore((state) => state.confirmManagedWorktreeStart);
  const dismiss = useSessionStore((state) => state.dismissManagedWorktreeStart);
  const preparing = useSessionStore((state) => state.worktreePreparing);
  if (!plan) return null;

  return (
    <ManagedWorktreeTransitionContent
      plan={plan}
      base={base}
      setBase={setBase}
      confirm={confirm}
      dismiss={dismiss}
      preparing={preparing}
    />
  );
}

export function ManagedWorktreeTransitionContent({
  plan,
  base,
  setBase,
  confirm,
  dismiss,
  preparing,
}: {
  plan: ProjectWorktreeTransitionPlan;
  base: ManagedWorktreeBase;
  setBase: (base: ManagedWorktreeBase) => void;
  confirm: () => Promise<void>;
  dismiss: () => void;
  preparing: boolean;
}) {
  const dialogRef = useModalFocus<HTMLElement>(true);
  const selected = plan.baseOptions.find((option) => option.id === base) ?? plan.baseOptions[0];
  const changes = changeSummary(plan.sourceChanges);
  const targetBranch = plan.action === "preserve_changes" ? plan.targetBranch : null;
  const selectedLabel = selected?.label ?? "the selected starting point";
  const currentBranch = plan.sourceBranch ?? "detached HEAD";
  const isBranchChange = Boolean(targetBranch);

  const handleKeyDown = (event: ReactKeyboardEvent<HTMLElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      dismiss();
      return;
    }
    if (event.key !== "Tab" || !dialogRef.current) return;
    const focusable = Array.from(dialogRef.current.querySelectorAll<HTMLElement>(
      "button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex='-1'])",
    ));
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = document.activeElement;
    if (event.shiftKey && (active === first || !dialogRef.current.contains(active))) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && (active === last || !dialogRef.current.contains(active))) {
      event.preventDefault();
      first.focus();
    }
  };

  return (
    <div
      className="fixed inset-0 z-elevated flex items-center justify-center bg-scrim p-4 backdrop-blur-[1px]"
      role="presentation"
    >
      <section
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="managed-worktree-title"
        tabIndex={-1}
        onKeyDown={handleKeyDown}
        className="w-full max-w-md rounded-2xl bg-bg-elevated p-5 shadow-lifted ring-1 ring-border-subtle"
      >
        <div className="flex items-start gap-3">
          <span className="grid size-9 shrink-0 place-items-center rounded-xl bg-checkout-worktree/10 text-checkout-worktree">
            <GitFork className="size-4" />
          </span>
          <div className="min-w-0">
            <h2 id="managed-worktree-title" className="text-base font-semibold text-ink">
              {isBranchChange
                ? `Open ${targetBranch} without moving your files?`
                : "Where should this chat work?"}
            </h2>
            <p className="mt-1 text-sm leading-5 text-ink-muted">
              This checkout has {changes}. Nothing will be stashed, committed, discarded, or
              moved automatically.
            </p>
          </div>
        </div>

        <div className="mt-4 grid grid-cols-[auto_1fr] gap-x-2 gap-y-1 rounded-xl bg-bg-sunken p-3 text-xs leading-5">
          <span className="text-ink-faint">Current work</span>
          <span className="min-w-0 truncate font-medium text-ink-secondary" title={plan.sourceRoot}>
            {currentBranch} · stays here
          </span>
          <span className="text-ink-faint">New chat</span>
          <span className="font-medium text-ink-secondary">
            {isBranchChange ? `${targetBranch} · new worktree` : "Choose below"}
          </span>
        </div>

        {targetBranch ? (
          <div className="mt-4 rounded-xl border border-accent/40 bg-accent-subtle p-3">
            <span className="flex items-center gap-2 text-sm font-medium text-ink">
              <GitBranch className="size-4 text-accent" /> {targetBranch}
            </span>
            <span className="mt-1 block text-xs leading-4 text-ink-muted">
              Opens in its own worktree. Your files on {currentBranch} stay exactly where they
              are.
            </span>
          </div>
        ) : (
          <fieldset className="mt-4">
            <legend className="mb-2 text-xs font-semibold text-ink-muted">
              Start the new worktree from
            </legend>
            <div className="space-y-2">
              {plan.baseOptions.map((option) => (
                <button
                  key={option.id}
                  type="button"
                  disabled={preparing}
                  onClick={() => setBase(option.id)}
                  className={
                    "flex w-full items-start gap-2.5 rounded-xl border p-3 text-left transition disabled:opacity-50 " +
                    (base === option.id
                      ? "border-accent bg-accent-subtle"
                      : "border-border-subtle hover:border-border")
                  }
                >
                  <span
                    className={
                      "mt-0.5 grid size-4 shrink-0 place-items-center rounded-full border " +
                      (base === option.id
                        ? "border-accent bg-accent text-on-accent"
                        : "border-ink-faint")
                    }
                  >
                    {base === option.id && <Check className="size-3" />}
                  </span>
                  <span className="min-w-0">
                    <span className="block text-sm font-medium text-ink">
                      {option.id === "current" ? currentBranch : option.reference}
                    </span>
                    <span className="mt-0.5 block text-xs leading-4 text-ink-muted">
                      {option.id === "current"
                        ? "Use the same committed starting point as this checkout."
                        : option.fallback
                          ? "The default branch is unavailable, so this currently uses the same commit."
                          : "Use the latest available default branch for unrelated work."}
                    </span>
                  </span>
                </button>
              ))}
            </div>
          </fieldset>
        )}

        <div className="mt-5 flex justify-end gap-2">
          <button
            type="button"
            disabled={preparing}
            onClick={dismiss}
            className="rounded-lg px-3 py-2 text-sm text-ink-muted transition hover:bg-bg-hover hover:text-ink disabled:opacity-50"
          >
            {isBranchChange ? "Cancel branch change" : "Work in this checkout"}
          </button>
          <button
            type="button"
            disabled={preparing || !selected}
            onClick={() => void confirm()}
            aria-label={
              targetBranch
                ? `Open ${targetBranch} in a new worktree`
                : `Create worktree from ${selectedLabel}`
            }
            title={
              targetBranch
                ? `Open ${targetBranch} in a new worktree`
                : `Create worktree from ${selectedLabel}`
            }
            className="flex items-center gap-2 rounded-lg bg-accent px-3 py-2 text-sm font-medium text-on-accent transition hover:bg-accent-hover disabled:opacity-50"
          >
            {preparing ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <ArrowRight className="size-3.5" />
            )}
            {targetBranch ? `Open ${targetBranch}` : "Create separate worktree"}
          </button>
        </div>
      </section>
    </div>
  );
}
