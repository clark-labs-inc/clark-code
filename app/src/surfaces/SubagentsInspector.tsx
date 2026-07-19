import { useEffect, useMemo, useState } from "react";
import { ArrowDownToLine, Check, CircleDashed, Loader2, TriangleAlert, X } from "lucide-react";
import { useReducedMotion } from "motion/react";
import { cn } from "../lib/cn";
import { useFanOutStore, type FanOutAgent } from "../store/fanOutStore";
import { FAN_OUT_STATUS_COPY, FanOutStatusIcon, fanOutSummary } from "./FanOutPanel";

export function formatElapsed(milliseconds: number): string {
  const seconds = Math.max(0, Math.floor(milliseconds / 1000));
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds % 60;
  return `${String(minutes).padStart(2, "0")}:${String(remainder).padStart(2, "0")}`;
}

function useClock(active: boolean): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [active]);
  return now;
}

function fallbackActivity(agent: FanOutAgent): string {
  if (agent.status === "queued") return "Waiting to start";
  if (agent.status === "running") return "Working on the delegated task";
  if (agent.status === "done") return "Complete";
  return "Needs attention";
}

function ProgressIcon({ state, reduce }: { state: "complete" | "current" | "queued" | "failed"; reduce: boolean | null }) {
  if (state === "complete") return <Check aria-hidden="true" className="size-3.5 text-success" strokeWidth={2.5} />;
  if (state === "failed") return <TriangleAlert aria-hidden="true" className="size-3.5 text-danger" />;
  if (state === "current") {
    return (
      <Loader2
        aria-hidden="true"
        className={cn("size-3.5 text-accent", !reduce && "animate-[spin_1s_linear_infinite]")}
      />
    );
  }
  return <CircleDashed aria-hidden="true" className="size-3.5 text-ink-faint" />;
}

function ProgressRow({
  label,
  detail,
  state,
  reduce,
  last = false,
}: {
  label: string;
  detail: string;
  state: "complete" | "current" | "queued" | "failed";
  reduce: boolean | null;
  last?: boolean;
}) {
  return (
    <li className="relative flex min-h-14 items-start gap-3 pb-3">
      {!last && <span aria-hidden="true" className="absolute left-[0.6875rem] top-6 h-[calc(100%-0.5rem)] w-px bg-border" />}
      <span
        className={cn(
          "relative z-10 mt-0.5 grid size-[1.4rem] shrink-0 place-items-center rounded-full border bg-bg",
          state === "current" ? "border-accent" : "border-border",
        )}
      >
        <ProgressIcon state={state} reduce={reduce} />
      </span>
      <span className="min-w-0 flex-1">
        <span className={cn("block text-sm", state === "current" ? "font-medium text-ink" : "text-ink-secondary")}>
          {label}
        </span>
        <span className="mt-0.5 block text-xs text-ink-faint">{detail}</span>
      </span>
    </li>
  );
}

export function SubagentsInspector() {
  const fanOut = useFanOutStore((state) => state.fanOut);
  const inspectorOpen = useFanOutStore((state) => state.inspectorOpen);
  const selectedAgentId = useFanOutStore((state) => state.selectedAgentId);
  const selectAgent = useFanOutStore((state) => state.selectAgent);
  const closeInspector = useFanOutStore((state) => state.closeInspector);
  const reduce = useReducedMotion();
  const running = fanOut?.agents.some((agent) => agent.status === "running") ?? false;
  const now = useClock(inspectorOpen && running);

  const selected = useMemo(
    () =>
      fanOut?.agents.find((agent) => agent.id === selectedAgentId) ??
      fanOut?.agents.find((agent) => agent.status === "running") ??
      fanOut?.agents[0],
    [fanOut, selectedAgentId],
  );

  useEffect(() => {
    if (!inspectorOpen) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") closeInspector();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [closeInspector, inspectorOpen]);

  if (!fanOut || !selected || !inspectorOpen) return null;

  const elapsed = selected.started_at_ms
    ? formatElapsed((selected.status === "running" ? now : (selected.updated_at_ms ?? now)) - selected.started_at_ms)
    : null;
  const status = FAN_OUT_STATUS_COPY[selected.status];
  const activity = selected.activity?.trim() || fallbackActivity(selected);
  const progressState =
    selected.status === "failed" ? "failed" : selected.status === "queued" ? "queued" : selected.status === "done" ? "complete" : "current";

  const jumpToTranscript = () => {
    closeInspector();
    requestAnimationFrame(() => {
      const target = document.getElementById("fan-out-panel");
      target?.scrollIntoView({ behavior: reduce ? "auto" : "smooth", block: "center" });
      target?.focus({ preventScroll: true });
    });
  };

  return (
    <aside aria-label="Subagents" className="flex min-h-0 min-w-0 flex-1 flex-col border-l border-border-subtle bg-bg">
      <header className="flex min-h-16 shrink-0 items-center gap-3 border-b border-border-subtle px-4">
        <div className="min-w-0 flex-1">
          <h2 className="text-base font-semibold text-ink">Subagents</h2>
          <p aria-live="polite" className="mt-0.5 truncate text-xs text-ink-muted">
            {fanOutSummary(fanOut)}
          </p>
        </div>
        <button
          type="button"
          onClick={closeInspector}
          aria-label="Close subagents"
          title="Close subagents (Esc)"
          className="grid size-11 shrink-0 place-items-center rounded-lg text-ink-muted outline-none transition hover:bg-bg-hover hover:text-ink focus-visible:ring-2 focus-visible:ring-accent"
        >
          <X aria-hidden="true" className="size-4" />
        </button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        <nav aria-label="Subagent tasks" className="space-y-1 border-b border-border-subtle p-3">
          {fanOut.agents.map((agent) => {
            const active = agent.id === selected.id;
            return (
              <button
                key={agent.id}
                type="button"
                aria-current={active ? "true" : undefined}
                onClick={() => selectAgent(agent.id)}
                className={cn(
                  "flex min-h-12 w-full items-center gap-3 rounded-lg border px-3 text-left outline-none transition",
                  active
                    ? "border-accent bg-accent-subtle ring-1 ring-accent/60"
                    : "border-transparent hover:border-border-subtle hover:bg-bg-hover",
                  "focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-bg",
                )}
              >
                <FanOutStatusIcon status={agent.status} reduce={reduce} className="size-4" />
                <span className="min-w-0 flex-1 truncate text-sm font-medium text-ink-secondary">{agent.label}</span>
                <span className="shrink-0 text-xs text-ink-faint">{FAN_OUT_STATUS_COPY[agent.status]}</span>
              </button>
            );
          })}
        </nav>

        <div className="p-5">
          <div className="border-b border-border-subtle pb-5">
            <h3 className="text-xl font-semibold leading-tight text-ink">{selected.label}</h3>
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <span
                className={cn(
                  "inline-flex min-h-7 items-center gap-1.5 rounded-full border px-2.5 text-xs font-medium",
                  selected.status === "running" && "border-accent/35 bg-accent-subtle text-accent",
                  selected.status === "done" && "border-success/35 bg-success/10 text-success",
                  selected.status === "queued" && "border-border bg-bg-secondary text-ink-muted",
                  selected.status === "failed" && "border-danger/35 bg-danger/10 text-danger",
                )}
              >
                <FanOutStatusIcon status={selected.status} reduce={reduce} className="size-3.5" />
                {status}{elapsed ? ` · ${elapsed}` : ""}
              </span>
              {selected.attempt && selected.attempt > 1 && (
                <span className="text-xs text-ink-faint">Attempt {selected.attempt}</span>
              )}
            </div>

            <div className="mt-5">
              <div className="text-[0.6875rem] font-semibold uppercase tracking-[0.12em] text-ink-faint">Objective</div>
              <p className="mt-2 text-sm leading-6 text-ink-secondary">{selected.objective || selected.label}</p>
            </div>

            <div className="mt-5">
              <div className="text-[0.6875rem] font-semibold uppercase tracking-[0.12em] text-ink-faint">Current activity</div>
              <p className="mt-2 text-sm leading-6 text-ink-secondary">{activity}</p>
            </div>
          </div>

          <div className="pt-5">
            <div className="mb-4 text-[0.6875rem] font-semibold uppercase tracking-[0.12em] text-ink-faint">Progress</div>
            <ol>
              <ProgressRow label="Task scoped" detail="Complete" state="complete" reduce={reduce} />
              <ProgressRow
                label={activity}
                detail={selected.status === "running" ? `Current${elapsed ? ` · ${elapsed}` : ""}` : status}
                state={progressState}
                reduce={reduce}
              />
              <ProgressRow
                label={selected.status === "failed" ? "Review the failure" : "Final report"}
                detail={selected.result ? "Ready" : selected.status === "failed" ? "Needs attention" : "Pending"}
                state={selected.result ? (selected.status === "failed" ? "failed" : "complete") : "queued"}
                reduce={reduce}
                last
              />
            </ol>
          </div>

          {selected.result && (
            <section aria-label="Subagent result" className="mt-1 border-t border-border-subtle pt-5">
              <div className="text-[0.6875rem] font-semibold uppercase tracking-[0.12em] text-ink-faint">Result</div>
              <p className="mt-2 whitespace-pre-wrap text-sm leading-6 text-ink-secondary">{selected.result}</p>
            </section>
          )}
        </div>
      </div>

      <footer className="shrink-0 border-t border-border-subtle p-3">
        <button
          type="button"
          onClick={jumpToTranscript}
          className="flex min-h-11 w-full items-center justify-center gap-2 rounded-lg border border-border text-sm font-medium text-ink-secondary outline-none transition hover:bg-bg-hover hover:text-ink focus-visible:ring-2 focus-visible:ring-accent"
        >
          <ArrowDownToLine aria-hidden="true" className="size-4" />
          Jump to transcript
        </button>
      </footer>
    </aside>
  );
}
