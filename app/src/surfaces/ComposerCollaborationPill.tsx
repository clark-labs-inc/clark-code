import { ListChecks, Play } from "lucide-react";
import { cn } from "../lib/cn";
import { useSessionStore } from "../store/sessionStore";

/** Explicit execution-vs-planning control, independent from approvals. */
export function ComposerCollaborationPill() {
  const mode = useSessionStore((state) => state.collaborationMode);
  const setMode = useSessionStore((state) => state.setCollaborationMode);
  const isLocalTarget = useSessionStore((state) =>
    state.session ? state.session.provider === "local" : state.activeProvider === "local",
  );
  if (!isLocalTarget) return null;

  const planning = mode === "plan";
  const Icon = planning ? ListChecks : Play;
  return (
    <button
      type="button"
      aria-pressed={planning}
      onClick={() => setMode(planning ? "default" : "plan")}
      title={planning ? "Leave Plan Mode" : "Enter read-only Plan Mode"}
      className={cn(
        "flex min-h-8 items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-medium transition duration-base ease-agent hover:bg-accent-subtle",
        planning ? "bg-accent-subtle text-accent" : "text-ink-secondary",
      )}
    >
      <Icon className="size-3.5" />
      {planning ? "Planning" : "Execute"}
    </button>
  );
}
