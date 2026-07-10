import { memo, useState } from "react";
import { motion, useReducedMotion } from "motion/react";
import {
  ChevronDown,
  ChevronRight,
  Circle,
  CircleCheck,
  CircleDot,
  ListChecks,
} from "lucide-react";
import { cn } from "../lib/cn";
import type { Plan, PlanPhaseStatus } from "../core-bridge/types";

const STATUS_ICON: Record<PlanPhaseStatus, typeof Circle> = {
  pending: Circle,
  in_progress: CircleDot,
  completed: CircleCheck,
};

const STATUS_CLASS: Record<PlanPhaseStatus, string> = {
  pending: "text-ink-faint",
  in_progress: "text-accent",
  completed: "text-ink-faint line-through decoration-ink-faint/60",
};

const EASE = [0.4, 0, 0.2, 1] as const;

/** Live checklist for the current plan — the render surface for the local
 *  agent's `update_plan` tool (and, for ACP/Clark providers, their own
 *  plan/execution-plan updates).
 *
 *  Wrapped in `memo` with a phase-content comparator: the parent (`Conversation`)
 *  re-renders on every streamed token and hands us a fresh `plan` object each
 *  frame, but the plan itself only changes on an `update_plan` tick — without
 *  this guard the animated card re-rendered ~60×/s during any run with an
 *  active plan (the streaming-flicker class this project has fought before). */
function PlanChecklistImpl({ plan }: { plan?: Plan }) {
  const reduce = useReducedMotion();
  const [manualExpansion, setManualExpansion] = useState<null | boolean>(null);
  if (!plan || plan.phases.length === 0) return null;

  const total = plan.phases.length;
  const done = plan.phases.filter((p) => p.status === "completed").length;
  const complete = done === total;
  const expanded = complete ? manualExpansion === true : true;
  const ToggleIcon = expanded ? ChevronDown : ChevronRight;

  return (
    <motion.div
      initial={reduce ? { opacity: 0 } : { opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.24, ease: EASE }}
      className="rounded-lg border border-border-subtle bg-bg-elevated px-3 py-2.5"
    >
      <button
        type="button"
        disabled={!complete}
        aria-expanded={expanded}
        onClick={() => complete && setManualExpansion((v) => !(v === true))}
        className={cn(
          "flex w-full min-w-0 items-center justify-between gap-3 text-left",
          complete && "rounded-md transition hover:bg-bg-hover/60",
        )}
      >
        <span className="flex min-w-0 items-center gap-2">
          <span className="grid size-7 shrink-0 place-items-center rounded-md bg-bg-secondary text-ink-muted">
            <ListChecks className="size-3.5" />
          </span>
          <span className="min-w-0">
            <span className="block text-sm font-semibold text-ink">
              {complete ? "Plan complete" : "Plan"}
            </span>
            <span className="block truncate font-mono text-xs tabular-nums text-ink-faint">
              {done}/{total}
            </span>
          </span>
        </span>
        {complete && (
          <span className="flex shrink-0 items-center gap-1 text-xs font-medium text-ink-muted">
            {expanded ? "Hide" : "Show"}
            <ToggleIcon className="size-3.5" />
          </span>
        )}
      </button>

      {expanded && (
        <ul className="mt-2 space-y-1.5">
          {plan.phases.map((phase, i) => {
            const Icon = STATUS_ICON[phase.status];
            return (
              <li key={i} className={cn("flex items-start gap-2 text-sm", STATUS_CLASS[phase.status])}>
                <Icon className="mt-0.5 size-3.5 shrink-0" />
                <span className="min-w-0 flex-1">{phase.title}</span>
              </li>
            );
          })}
        </ul>
      )}
    </motion.div>
  );
}

/** Skip re-render unless the plan's phases actually changed (count, order,
 *  titles, or statuses) — a new `plan` object reference with identical content
 *  arrives every streamed frame. */
export const PlanChecklist = memo(PlanChecklistImpl, (a, b) => {
  const pa = a.plan?.phases;
  const pb = b.plan?.phases;
  if (pa === pb) return true;
  if (!pa || !pb || pa.length !== pb.length) return false;
  return pa.every((p, i) => p.title === pb[i].title && p.status === pb[i].status);
});
