import { useState } from "react";
import { History, Check, Loader2, ChevronDown } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { restoreCheckpoint } from "../lib/checkpoint";
import { cn } from "../lib/cn";
import type { RunView } from "../core-bridge/types";

type State =
  | { kind: "idle" }
  | { kind: "confirm"; run: RunView; index: number }
  | { kind: "busy" }
  | { kind: "done" }
  | { kind: "error"; msg: string };

/** Restore the whole working tree to any earlier run's checkpoint, not just
 *  the latest (that's `UndoBar`'s job). Every run's checkpoint SHA already
 *  lives in `snapshot.runs` — this just exposes picking one that isn't the
 *  most recent. Jumping back is a bigger action than a single undo (it
 *  deletes files created by every turn since), so it's tucked behind an
 *  explicit picker with a confirmation, not a one-click button. */
export function RewindPicker({ excludeSha }: { excludeSha?: string }) {
  const cwd = useSessionStore((s) => s.localSettings.cwd);
  // Narrow to `runs` only — this picker doesn't need to re-render on unrelated
  // snapshot churn (it's only mounted when idle, but keep the discipline).
  const runsMap = useSessionStore((s) => s.snapshot.runs);
  const [open, setOpen] = useState(false);
  const [state, setState] = useState<State>({ kind: "idle" });

  const runs = Object.values(runsMap).filter(
    (r) =>
      r.checkpoint &&
      r.status !== "running" &&
      r.status !== "queued" &&
      r.checkpoint !== excludeSha,
  );
  if (runs.length === 0) return null;

  const restore = async (run: RunView) => {
    setState({ kind: "busy" });
    try {
      await restoreCheckpoint(cwd, run.checkpoint!);
      setState({ kind: "done" });
    } catch (e) {
      setState({ kind: "error", msg: e instanceof Error ? e.message : String(e) });
    }
  };

  if (state.kind === "done") {
    return (
      <div className="flex items-center gap-2 pl-[1.4rem] text-xs text-ink-muted">
        <Check className="size-3.5 text-success" />
        Restored to that point — files from later turns were removed. Re-send to reapply.
      </div>
    );
  }

  return (
    <div className="relative pl-[1.4rem]">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        title="Restore the whole project to an earlier point in this conversation"
        className="flex items-center gap-1.5 rounded-md px-2 py-1 text-xs font-medium text-ink-muted transition hover:bg-bg-hover hover:text-ink"
      >
        <History className="size-3.5" />
        Restore an earlier point
        <ChevronDown className="size-3 opacity-70" />
      </button>

      {open && (
        <div
          role="menu"
          className="popover-surface absolute bottom-full left-[1.4rem] z-30 mb-2 w-64 rounded-xl bg-bg-elevated p-1 shadow-lg ring-1 ring-border-subtle"
        >
          <div className="px-2.5 py-1.5 text-[0.7rem] font-medium uppercase tracking-wide text-ink-faint">
            Jump to before…
          </div>
          {runs.map((r, i) => (
            <button
              key={r.id}
              type="button"
              role="menuitem"
              onClick={() => setState({ kind: "confirm", run: r, index: i + 1 })}
              className="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-sm text-ink transition hover:bg-bg-hover"
            >
              Turn {i + 1}
            </button>
          ))}
        </div>
      )}

      {state.kind === "confirm" && (
        <div className="mt-2 max-w-xs rounded-lg border border-warning/30 bg-warning/8 p-2.5 text-xs text-ink-secondary">
          This removes files created or changed by every turn after Turn {state.index} on
          disk. It's a jump, not a single undo.
          <div className="mt-2 flex gap-2">
            <button
              type="button"
              onClick={() => {
                setOpen(false);
                void restore(state.run);
              }}
              className="rounded-md bg-danger/12 px-2.5 py-1 font-medium text-danger transition hover:bg-danger/20"
            >
              Restore anyway
            </button>
            <button
              type="button"
              onClick={() => setState({ kind: "idle" })}
              className="rounded-md px-2 py-1 text-ink-muted transition hover:bg-bg-hover"
            >
              Cancel
            </button>
          </div>
        </div>
      )}
      {state.kind === "busy" && (
        <div className={cn("mt-2 flex items-center gap-1.5 text-xs text-ink-muted")}>
          <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite]" /> Restoring…
        </div>
      )}
      {state.kind === "error" && <div className="mt-2 text-xs text-danger">{state.msg}</div>}
    </div>
  );
}
