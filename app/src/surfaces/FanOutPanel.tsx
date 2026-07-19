import { useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { Check, ChevronDown, Circle, Loader2, Network, TriangleAlert } from "lucide-react";
import { cn } from "../lib/cn";
import { DUR, EASE } from "../lib/motion";
import { useFanOutStore, type FanOut, type FanOutAgent } from "../store/fanOutStore";

const STATUS_COPY: Record<FanOutAgent["status"], string> = {
  done: "Ready",
  running: "Working",
  queued: "Waiting",
  failed: "Needs attention",
};

function StatusIcon({ status }: { status: FanOutAgent["status"] }) {
  if (status === "done") return <Check className="size-3.5 text-success" strokeWidth={2.5} />;
  if (status === "failed") return <TriangleAlert className="size-3.5 text-danger" />;
  if (status === "running") {
    return <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite] text-accent" />;
  }
  return <Circle className="size-3.5 text-ink-faint" strokeDasharray="3 3" />;
}

export function fanOutSummary(fanOut: FanOut): string {
  const failed = fanOut.agents.filter((agent) => agent.status === "failed").length;
  if (failed > 0) {
    return `${failed} part${failed === 1 ? " needs" : "s need"} attention`;
  }
  if (fanOut.done >= fanOut.total && fanOut.total > 0) return "All parts are ready";
  if (fanOut.running > 0) return `${fanOut.done} of ${fanOut.total} parts ready`;
  return `${fanOut.total} parts waiting`;
}

export function FanOutCard({ fanOut, reduce }: { fanOut: FanOut; reduce: boolean | null }) {
  const [expanded, setExpanded] = useState(false);
  const shown = fanOut.agents.slice(0, 8);
  const more = Math.max(0, fanOut.total - shown.length);
  const summary = fanOutSummary(fanOut);

  return (
    <motion.div
      initial={reduce ? { opacity: 0 } : { opacity: 0, y: 8, scale: 0.99 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={reduce ? { opacity: 0 } : { opacity: 0, y: -6, scale: 0.98 }}
      transition={{ duration: DUR.base, ease: EASE.out }}
      className="overflow-hidden rounded-xl border border-border-subtle bg-bg-elevated"
    >
      <button
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((open) => !open)}
        className="fan-out-toggle group flex w-full items-center gap-3 px-3.5 py-3 text-left transition-colors hover:bg-bg-hover"
      >
        <Network className="size-4 shrink-0 text-accent" />
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium text-ink">Parallel work</div>
          <div className="mt-0.5 truncate text-xs text-ink-muted">{summary}</div>
        </div>
        <ChevronDown
          className={cn("size-3.5 text-ink-faint transition-transform", expanded && "rotate-180")}
        />
      </button>

      <AnimatePresence initial={false}>
        {expanded && (
          <motion.div
            initial={reduce ? { opacity: 0 } : { height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={reduce ? { opacity: 0 } : { height: 0, opacity: 0 }}
            transition={{ duration: DUR.base, ease: EASE.out }}
            className="overflow-hidden"
          >
            <div className="space-y-0.5 p-2">
              {shown.map((agent) => (
                <div
                  key={agent.id}
                  className={cn(
                    "flex min-w-0 items-center gap-2.5 rounded-lg px-2 py-2",
                    agent.status === "running" && "bg-accent/8",
                  )}
                >
                  <span className="grid size-5 shrink-0 place-items-center">
                    <StatusIcon status={agent.status} />
                  </span>
                  <span className="min-w-0 flex-1 truncate text-sm text-ink-secondary">
                    {agent.label}
                  </span>
                  <span className="shrink-0 text-xs text-ink-faint">
                    {STATUS_COPY[agent.status]}
                  </span>
                </div>
              ))}
              {more > 0 && (
                <div className="px-2 py-1.5 text-xs text-ink-faint">+{more} more parts</div>
              )}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.div>
  );
}

/** A deliberately quiet parallel-work surface. The aggregate state is visible
 *  at a glance; agent-by-agent detail stays collapsed unless the user asks. */
export function FanOutPanel() {
  const fanOut = useFanOutStore((s) => s.fanOut);
  const reduce = useReducedMotion();
  return (
    <AnimatePresence initial={false}>
      {fanOut && <FanOutCard key="fanout" fanOut={fanOut} reduce={reduce} />}
    </AnimatePresence>
  );
}
