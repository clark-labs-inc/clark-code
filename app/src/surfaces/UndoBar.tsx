import { useState } from "react";
import { Undo2, Check, Loader2 } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { restoreCheckpoint } from "../lib/checkpoint";
import { cn } from "../lib/cn";

type State = { kind: "idle" } | { kind: "busy" } | { kind: "done" } | { kind: "error"; msg: string };

/** One-click revert of the working tree to the snapshot taken before this run's
 *  edits landed. Shown after a completed run that has a checkpoint + made edits. */
export function UndoBar({ sha }: { sha: string }) {
  const cwd = useSessionStore((s) => s.activeProjectRoot ?? "");
  const remote = useSessionStore((s) => s.activeRemote);
  const [state, setState] = useState<State>({ kind: "idle" });

  const undo = async () => {
    setState({ kind: "busy" });
    try {
      await restoreCheckpoint(cwd, sha, remote);
      setState({ kind: "done" });
    } catch (e) {
      setState({ kind: "error", msg: e instanceof Error ? e.message : String(e) });
    }
  };

  if (state.kind === "done") {
    return (
      <div className="flex items-center gap-2 pl-[1.4rem] text-xs text-ink-muted">
        <Check className="size-3.5 text-success" />
        Changes from this run were reverted on disk. Re-send to reapply.
      </div>
    );
  }

  return (
    <div className="flex items-center gap-2 pl-[1.4rem]">
      <button
        onClick={() => void undo()}
        disabled={state.kind === "busy"}
        title="Revert the files this run changed, back to before it started"
        className={cn(
          "flex items-center gap-1.5 rounded-md px-2 py-1 text-xs font-medium transition",
          "text-ink-muted hover:bg-bg-hover hover:text-ink disabled:opacity-60",
        )}
      >
        {state.kind === "busy" ? (
          <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite]" />
        ) : (
          <Undo2 className="size-3.5" />
        )}
        Undo changes
      </button>
      {state.kind === "error" && <span className="text-xs text-danger">{state.msg}</span>}
    </div>
  );
}
