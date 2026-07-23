import { motion, AnimatePresence, useReducedMotion } from "motion/react";
import {
  Check,
  ChevronRight,
  CircleDashed,
  Loader2,
  Network,
  TriangleAlert,
} from "lucide-react";
import { cn } from "../lib/cn";
import { DUR, EASE } from "../lib/motion";
import { useFanOutStore, type FanOut, type FanOutAgent } from "../store/fanOutStore";

export const FAN_OUT_STATUS_COPY: Record<FanOutAgent["status"], string> = {
  done: "Complete",
  running: "Running",
  queued: "Queued",
  failed: "Needs attention",
};

export function FanOutStatusIcon({
  status,
  reduce = false,
  className,
}: {
  status: FanOutAgent["status"];
  reduce?: boolean | null;
  className?: string;
}) {
  if (status === "done") {
    return <Check aria-hidden="true" className={cn("size-5 text-success", className)} strokeWidth={2.5} />;
  }
  if (status === "failed") {
    return <TriangleAlert aria-hidden="true" className={cn("size-5 text-danger", className)} />;
  }
  if (status === "running") {
    return (
      <Loader2
        aria-hidden="true"
        className={cn("size-5 text-accent", !reduce && "animate-[spin_1s_linear_infinite]", className)}
      />
    );
  }
  return <CircleDashed aria-hidden="true" className={cn("size-5 text-ink-faint", className)} />;
}

export function fanOutSummary(fanOut: FanOut): string {
  const running = fanOut.agents.filter((agent) => agent.status === "running").length;
  const done = fanOut.agents.filter((agent) => agent.status === "done").length;
  const failed = fanOut.agents.filter((agent) => agent.status === "failed").length;
  const queued = fanOut.agents.filter((agent) => agent.status === "queued").length;
  const parts: string[] = [];
  if (running > 0) parts.push(`${running} running`);
  if (done > 0) parts.push(`${done} complete`);
  if (queued > 0) parts.push(`${queued} queued`);
  if (failed > 0) parts.push(`${failed} need${failed === 1 ? "s" : ""} attention`);
  return parts.join(" · ") || `${fanOut.total} waiting`;
}

export function FanOutCard({
  fanOut,
  reduce,
  selectedAgentId = null,
  inspectorOpen = false,
  onSelect,
}: {
  fanOut: FanOut;
  reduce: boolean | null;
  selectedAgentId?: string | null;
  inspectorOpen?: boolean;
  onSelect?: (agentId: string) => void;
}) {
  const shown = fanOut.agents.slice(0, 8);
  const more = Math.max(0, fanOut.total - shown.length);

  return (
    <motion.section
      id="fan-out-panel"
      tabIndex={-1}
      aria-label="Parallel subagent work"
      initial={reduce ? { opacity: 0 } : { opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      exit={reduce ? { opacity: 0 } : { opacity: 0, y: -4 }}
      transition={{ duration: DUR.base, ease: EASE.out }}
      className="scroll-mt-6"
    >
      <div className="mb-2 flex min-w-0 items-center gap-2 text-xs text-ink-muted">
        <Network aria-hidden="true" className="size-3.5 shrink-0 text-accent" />
        <span className="font-medium text-ink-secondary">Parallel work</span>
        <span aria-live="polite" className="min-w-0 truncate text-ink-faint">
          {fanOutSummary(fanOut)}
        </span>
      </div>

      <div className="flex flex-wrap gap-2">
        {shown.map((agent) => {
          const selected = inspectorOpen && selectedAgentId === agent.id;
          const status = FAN_OUT_STATUS_COPY[agent.status];
          return (
            <button
              key={agent.id}
              type="button"
              data-agent-id={agent.id}
              aria-label={`${agent.label}, ${status}. Open subagent details.`}
              aria-pressed={selected}
              onClick={() => onSelect?.(agent.id)}
              className={cn(
                "group flex min-h-14 min-w-[12rem] flex-1 items-center gap-2.5 rounded-lg border bg-bg-elevated px-3 py-2 text-left outline-none transition",
                "border-border-subtle hover:border-border hover:bg-bg-hover focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-bg",
                selected && "border-accent bg-accent-subtle ring-1 ring-accent/70",
              )}
            >
              <span
                className={cn(
                  "grid size-7 shrink-0 place-items-center rounded-full border border-border",
                  agent.status === "running" && "border-accent/55 bg-accent-subtle",
                  agent.status === "done" && "border-success/45 bg-success/10",
                  agent.status === "failed" && "border-danger/45 bg-danger/10",
                )}
              >
                <FanOutStatusIcon status={agent.status} reduce={reduce} className="size-4" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block break-words text-sm font-medium leading-4 text-ink-secondary">
                  {agent.label}
                </span>
                <span className="block text-xs text-ink-faint">{status}</span>
              </span>
              <ChevronRight
                aria-hidden="true"
                className="size-3.5 shrink-0 text-ink-faint transition-transform group-hover:translate-x-0.5"
              />
            </button>
          );
        })}
        {more > 0 && (
          <div className="flex min-h-11 items-center px-2 text-xs text-ink-faint">+{more} more</div>
        )}
      </div>
    </motion.section>
  );
}

export function FanOutPanel() {
  const fanOut = useFanOutStore((state) => state.fanOut);
  const inspectorOpen = useFanOutStore((state) => state.inspectorOpen);
  const selectedAgentId = useFanOutStore((state) => state.selectedAgentId);
  const openInspector = useFanOutStore((state) => state.openInspector);
  const reduce = useReducedMotion();

  return (
    <AnimatePresence initial={false}>
      {fanOut && (
        <FanOutCard
          key="fanout"
          fanOut={fanOut}
          reduce={reduce}
          selectedAgentId={selectedAgentId}
          inspectorOpen={inspectorOpen}
          onSelect={openInspector}
        />
      )}
    </AnimatePresence>
  );
}
