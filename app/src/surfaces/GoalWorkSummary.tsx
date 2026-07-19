import { useEffect, useState, type ReactNode } from "react";
import { ChevronRight } from "lucide-react";
import type { GoalState } from "../core-bridge/types";
import { formatGoalDuration, goalElapsedSeconds } from "../lib/goal";

export function GoalWorkSummary({ goal, children }: { goal: GoalState; children: ReactNode }) {
  const [open, setOpen] = useState(goal.status === "active");
  const [, tick] = useState(0);

  useEffect(() => setOpen(goal.status === "active"), [goal.id, goal.status]);
  useEffect(() => {
    if (goal.status !== "active") return;
    const id = window.setInterval(() => tick((value) => value + 1), 1_000);
    return () => window.clearInterval(id);
  }, [goal.status]);

  return (
    <section className="border-b border-border-subtle pb-3" aria-label="Goal work receipt">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
        className="group flex min-h-9 w-full items-center gap-1.5 text-left text-base font-medium text-ink-muted transition-colors hover:text-ink"
      >
        <span>{goal.status === "active" ? "Working for" : "Worked for"}</span>
        <span className="tabular-nums">{formatGoalDuration(goalElapsedSeconds(goal))}</span>
        <ChevronRight
          className={`size-4 transition-transform duration-200 ${open ? "rotate-90" : ""}`}
          aria-hidden
        />
      </button>
      {open && <div className="mt-2 flex flex-col gap-4">{children}</div>}
    </section>
  );
}
