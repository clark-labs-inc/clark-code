import { Network } from "lucide-react";
import { cn } from "../lib/cn";
import { useFanOutStore, type FanOutAgent } from "../store/fanOutStore";

const STATUS_DOT: Record<FanOutAgent["status"], string> = {
  done: "bg-success",
  running: "bg-info",
  queued: "bg-ink-faint",
  failed: "bg-danger",
};

/** The parallel fan-out surface: gives "thousands of cloud agents" a face — a
 *  live swarm of agent tiles with an aggregate progress readout. Renders nothing
 *  unless a fan-out is active (see fanOutStore integration note). */
export function FanOutPanel() {
  const fanOut = useFanOutStore((s) => s.fanOut);
  if (!fanOut) return null;

  const pct = fanOut.total > 0 ? Math.round((fanOut.done / fanOut.total) * 100) : 0;
  const shown = fanOut.agents.slice(0, 15);
  const more = fanOut.total - shown.length;

  return (
    <div className="rounded-xl border border-border-subtle bg-bg-elevated p-4">
      <div className="flex items-center gap-3">
        <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-accent text-on-accent">
          <Network className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-semibold text-ink">
            Fanned out to {fanOut.total.toLocaleString()} agents
          </div>
          <div className="truncate text-xs text-ink-muted">{fanOut.title}</div>
        </div>
      </div>

      <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-bg-tertiary">
        <div
          className="h-full rounded-full bg-accent transition-all"
          style={{ width: `${Math.max(3, pct)}%` }}
        />
      </div>
      <div className="mt-1.5 font-mono text-[0.7rem] text-ink-faint tabular-nums">
        {fanOut.done} done · {fanOut.running} running · merging as they finish
      </div>

      <div className="mt-3 grid grid-cols-4 gap-1.5 sm:grid-cols-8">
        {shown.map((a) => (
          <div
            key={a.id}
            className={cn(
              "flex aspect-[1.35/1] flex-col justify-between rounded-md border border-border-subtle bg-bg-secondary px-1.5 py-1",
              a.status === "queued" && "opacity-50",
            )}
          >
            <span className={cn("size-1.5 rounded-full", STATUS_DOT[a.status])} />
            <span className="truncate font-mono text-[0.55rem] leading-tight text-ink-faint">
              {a.label}
            </span>
          </div>
        ))}
        {more > 0 && (
          <div className="grid aspect-[1.35/1] place-items-center rounded-md bg-bg-sunken font-mono text-[0.65rem] text-ink-muted">
            +{more}
          </div>
        )}
      </div>
    </div>
  );
}
