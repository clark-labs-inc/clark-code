import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { Network } from "lucide-react";
import { cn } from "../lib/cn";
import { useFanOutStore, type FanOut, type FanOutAgent } from "../store/fanOutStore";

const STATUS_DOT: Record<FanOutAgent["status"], string> = {
  done: "bg-success",
  running: "bg-info",
  queued: "bg-ink-faint",
  failed: "bg-danger",
};

const EASE = [0.4, 0, 0.2, 1] as const;

function FanOutCard({ fanOut, reduce }: { fanOut: FanOut; reduce: boolean | null }) {
  const pct = fanOut.total > 0 ? Math.round((fanOut.done / fanOut.total) * 100) : 0;
  const shown = fanOut.agents.slice(0, 15);
  const more = fanOut.total - shown.length;

  return (
    <motion.div
      initial={reduce ? { opacity: 0 } : { opacity: 0, y: 8, scale: 0.99 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={reduce ? { opacity: 0 } : { opacity: 0, y: -6, scale: 0.98 }}
      transition={{ duration: 0.28, ease: EASE }}
      className="overflow-hidden rounded-xl border border-border-subtle bg-bg-elevated p-4"
    >
      <div className="flex items-center gap-3">
        <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-accent text-on-accent">
          <Network className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-semibold text-ink">
            Fanned out to {fanOut.total.toLocaleString()} agents
          </div>
          {fanOut.title && <div className="truncate text-xs text-ink-muted">{fanOut.title}</div>}
        </div>
      </div>

      <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-bg-tertiary">
        <motion.div
          className="h-full rounded-full bg-accent"
          initial={false}
          animate={{ width: `${Math.max(3, pct)}%` }}
          transition={{ duration: 0.5, ease: EASE }}
        />
      </div>
      <div className="mt-1.5 font-mono text-[0.7rem] tabular-nums text-ink-faint">
        {fanOut.done} done · {fanOut.running} running · merging as they finish
      </div>

      <div className="mt-3 grid grid-cols-4 gap-1.5 sm:grid-cols-8">
        <AnimatePresence initial={false}>
          {shown.map((a, i) => (
            <motion.div
              key={a.id}
              layout
              initial={reduce ? false : { opacity: 0, scale: 0.85 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.85 }}
              transition={{
                duration: 0.2,
                delay: reduce ? 0 : Math.min(i * 0.015, 0.2),
                ease: EASE,
              }}
              className={cn(
                "flex aspect-[1.35/1] flex-col justify-between rounded-md border border-border-subtle bg-bg-secondary px-1.5 py-1",
                a.status === "queued" && "opacity-50",
              )}
            >
              <motion.span
                className={cn("size-1.5 rounded-full", STATUS_DOT[a.status])}
                animate={
                  a.status === "running" && !reduce ? { opacity: [1, 0.35, 1] } : { opacity: 1 }
                }
                transition={
                  a.status === "running"
                    ? { duration: 1.4, repeat: Infinity, ease: "easeInOut" }
                    : { duration: 0.2 }
                }
              />
              <span className="truncate font-mono text-[0.55rem] leading-tight text-ink-faint">
                {a.label}
              </span>
            </motion.div>
          ))}
        </AnimatePresence>
        {more > 0 && (
          <div className="grid aspect-[1.35/1] place-items-center rounded-md bg-bg-sunken font-mono text-[0.65rem] text-ink-muted">
            +{more}
          </div>
        )}
      </div>
    </motion.div>
  );
}

/** The parallel fan-out surface: gives "thousands of cloud agents" a face — a
 *  live swarm of agent tiles with an aggregate progress readout. Fades in when a
 *  fan-out starts and fades out when the run ends; renders nothing otherwise. */
export function FanOutPanel() {
  const fanOut = useFanOutStore((s) => s.fanOut);
  const reduce = useReducedMotion();
  return (
    <AnimatePresence initial={false}>
      {fanOut && <FanOutCard key="fanout" fanOut={fanOut} reduce={reduce} />}
    </AnimatePresence>
  );
}
