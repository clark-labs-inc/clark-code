import { useEffect, useMemo, useState } from "react";
import { Files, Target, X } from "lucide-react";
import { summarizeEdits } from "../lib/diff";
import { formatGoalDuration, goalElapsedSeconds, goalStatusLabel } from "../lib/goal";
import { useSessionStore } from "../store/sessionStore";

type OpenReceipt = "changes" | "goal" | null;

function compactNumber(value: number): string {
  return value >= 1_000 ? `${(value / 1_000).toFixed(value >= 10_000 ? 0 : 1)}K` : String(value);
}

export function GoalStatusRail() {
  const goal = useSessionStore((state) => state.snapshot.goal);
  const calls = useSessionStore((state) => state.snapshot.tool_calls);
  const timeline = useSessionStore((state) => state.snapshot.timeline);
  const [open, setOpen] = useState<OpenReceipt>(null);
  const [, tick] = useState(0);
  const allCalls = useMemo(() => {
    if (!goal?.run) return Object.values(calls);
    const ids = new Set(
      timeline.flatMap((item) => item.item === "tool_call" && item.run === goal.run ? [item.id] : []),
    );
    return Object.values(calls).filter((call) => ids.has(call.id));
  }, [calls, goal?.run, timeline]);
  const edits = useMemo(() => summarizeEdits(allCalls), [allCalls]);
  const paths = useMemo(
    () => [...new Set(allCalls.flatMap((call) => call.locations.map((location) => location.path)))],
    [allCalls],
  );

  useEffect(() => {
    if (goal?.status !== "active") return;
    const id = window.setInterval(() => tick((value) => value + 1), 1_000);
    return () => window.clearInterval(id);
  }, [goal?.status]);

  useEffect(() => setOpen(null), [goal?.id]);
  if (!goal && !edits) return null;

  const receipt = open === "goal" && goal ? (
    <div>
      <p className="text-sm font-medium text-ink">{goal.objective}</p>
      <dl className="mt-3 grid grid-cols-2 gap-x-6 gap-y-2 text-xs">
        <div><dt className="text-ink-faint">Status</dt><dd className="mt-0.5 text-ink-secondary">{goalStatusLabel(goal)}</dd></div>
        <div><dt className="text-ink-faint">Elapsed</dt><dd className="mt-0.5 tabular-nums text-ink-secondary">{formatGoalDuration(goalElapsedSeconds(goal))}</dd></div>
        <div><dt className="text-ink-faint">Goal turns</dt><dd className="mt-0.5 tabular-nums text-ink-secondary">{goal.continuations}</dd></div>
        <div><dt className="text-ink-faint">Tokens used</dt><dd className="mt-0.5 tabular-nums text-ink-secondary">{compactNumber(goal.tokens_used)}{goal.token_budget ? ` / ${compactNumber(goal.token_budget)}` : ""}</dd></div>
      </dl>
      {goal.blocker_reason && (
        <p className="mt-3 rounded-lg bg-warning/8 px-3 py-2 text-xs leading-relaxed text-warning">
          {goal.blocker_reason}
        </p>
      )}
    </div>
  ) : open === "changes" && edits ? (
    <div>
      <p className="text-sm font-medium text-ink">
        {edits.files} file{edits.files === 1 ? "" : "s"} changed
        <span className="ml-2 font-mono text-xs"><span className="text-success">+{edits.adds}</span>{" "}<span className="text-danger">−{edits.dels}</span></span>
      </p>
      {paths.length > 0 && (
        <ul className="mt-2 max-h-32 space-y-1 overflow-y-auto font-mono text-xs text-ink-muted">
          {paths.slice(0, 8).map((path) => <li key={path} className="truncate">{path}</li>)}
        </ul>
      )}
    </div>
  ) : null;

  return (
    <div className="relative z-20 mx-auto w-full max-w-2xl shrink-0 px-3 pb-2 sm:px-5">
      {receipt && (
        <div className="absolute inset-x-3 bottom-full mb-2 rounded-xl border border-border bg-bg-elevated p-4 shadow-lifted sm:inset-x-5">
          <button type="button" onClick={() => setOpen(null)} aria-label="Close receipt" className="absolute right-2 top-2 grid size-7 place-items-center rounded-md text-ink-faint hover:bg-bg-hover hover:text-ink">
            <X className="size-3.5" />
          </button>
          {receipt}
        </div>
      )}
      <div className="flex items-center gap-2 overflow-x-auto [scrollbar-width:none]">
        {edits && (
          <button type="button" onClick={() => setOpen((value) => value === "changes" ? null : "changes")} aria-expanded={open === "changes"} className="flex h-9 shrink-0 items-center gap-1.5 rounded-full border border-border bg-bg-elevated px-2.5 text-xs font-medium text-ink-secondary shadow-sm transition hover:bg-bg-hover hover:text-ink sm:gap-2 sm:px-3.5">
            <Files className="size-3.5" aria-hidden />
            <span>{edits.files} file{edits.files === 1 ? "" : "s"}</span>
            <span className="font-mono tabular-nums"><span className="text-success">+{compactNumber(edits.adds)}</span>{" "}<span className="text-danger">−{compactNumber(edits.dels)}</span></span>
          </button>
        )}
        {goal && (
          <button type="button" onClick={() => setOpen((value) => value === "goal" ? null : "goal")} aria-expanded={open === "goal"} className="flex h-9 shrink-0 items-center gap-1.5 rounded-full border border-border bg-bg-elevated px-2.5 text-xs font-medium text-ink-secondary shadow-sm transition hover:bg-bg-hover hover:text-ink sm:gap-2 sm:px-3.5">
            <Target className={goal.status === "active" ? "size-3.5 text-accent" : "size-3.5"} aria-hidden />
            <span>{goalStatusLabel(goal)}</span>
            <span className="tabular-nums text-ink-faint">{formatGoalDuration(goalElapsedSeconds(goal))}</span>
          </button>
        )}
      </div>
    </div>
  );
}
