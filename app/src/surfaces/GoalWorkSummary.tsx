import { useEffect, useState, type ReactNode } from "react";
import { ChevronRight } from "lucide-react";
import type { GoalState } from "../core-bridge/types";
import { formatGoalDuration, goalElapsedSeconds } from "../lib/goal";

export function GoalWorkSummary({
  goal,
  runActive,
  children,
}: {
  goal: GoalState;
  runActive: boolean;
  children: ReactNode;
}) {
  const working = goal.status === "active" || runActive;
  const [open, setOpen] = useState(working);
  const [, tick] = useState(0);

  useEffect(() => setOpen(working), [goal.id, working]);
  useEffect(() => {
    if (!working) return;
    const id = window.setInterval(() => tick((value) => value + 1), 1_000);
    return () => window.clearInterval(id);
  }, [working]);

  return (
    <section className="border-b border-border-subtle pb-3" aria-label="Goal work receipt">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
        className="group flex min-h-9 w-full items-center gap-1.5 text-left text-base font-medium text-ink-muted transition-colors hover:text-ink"
      >
        <span>{working ? "Working for" : "Worked for"}</span>
        <span className="tabular-nums">
          {formatGoalDuration(goalElapsedSeconds(goal, Date.now(), working))}
        </span>
        <ChevronRight
          className={`size-4 transition-transform duration-200 ${open ? "rotate-90" : ""}`}
          aria-hidden
        />
      </button>
      {open && <div className="mt-2 flex flex-col gap-4">{children}</div>}
    </section>
  );
}
