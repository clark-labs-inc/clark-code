import { Check, Circle, Loader2, TriangleAlert, X } from "lucide-react";
import { useReducedMotion } from "motion/react";

import type { ToolCall, ToolStatus } from "../../core-bridge/types";
import type { Activity } from "../../lib/activity";
import { lastProgressLine } from "../../lib/activity";
import { cn } from "../../lib/cn";
import { specProgressTitle } from "../../lib/specProgress";
import { ResearchOutline } from "../work/ResearchWork";

interface SpecRunProgressProps {
  activity: Activity;
  calls: readonly ToolCall[];
  compact?: boolean;
}

function StatusIcon({ status }: { status: ToolStatus }) {
  const reduceMotion = useReducedMotion();
  if (status === "completed") return <Check aria-hidden className="size-3.5 text-success" />;
  if (status === "failed") return <TriangleAlert aria-hidden className="size-3.5 text-danger" />;
  if (status === "cancelled") return <X aria-hidden className="size-3.5 text-ink-faint" />;
  if (status === "in_progress") {
    return (
      <Loader2
        aria-hidden
        className={cn("size-3.5 text-accent", reduceMotion ? "breathe" : "animate-[spin_1s_linear_infinite]")}
      />
    );
  }
  return <Circle aria-hidden className="size-3 text-ink-faint" />;
}

function stepDetail(call: ToolCall): string | undefined {
  return call.progress?.latest_activity || lastProgressLine(call) || call.locations[0]?.path;
}

export function SpecRunProgress({ activity, calls, compact = false }: SpecRunProgressProps) {
  const visibleCalls = calls.slice(compact ? -4 : -6);
  const activeResearch = [...visibleCalls].reverse().find(
    (call) => call.status === "in_progress" && call.kind === "research",
  );
  const completed = calls.filter((call) => call.status === "completed").length;

  if (compact) {
    return (
      <section
        data-qa="spec-run-progress"
        aria-label="Live Spec progress"
        className="mx-auto mb-5 max-w-[44rem] rounded-xl border border-border-subtle bg-bg-secondary/80 px-4 py-3 shadow-sm"
      >
        <div className="flex min-w-0 items-center gap-3">
          <span className="size-2 shrink-0 rounded-full bg-accent breathe" aria-hidden />
          <div className="min-w-0 flex-1">
            <div className="flex items-center justify-between gap-3">
              <p className="truncate text-sm font-medium text-ink">{activity.label}</p>
              <span className="shrink-0 text-xs tabular-nums text-ink-faint">
                {calls.length > 0 ? `${completed} of ${calls.length} updates` : "Starting"}
              </span>
            </div>
            {activity.progress !== undefined && (
              <div className="mt-2 h-1 overflow-hidden rounded-full bg-bg-tertiary">
                <div
                  className="h-full rounded-full bg-accent transition-[width] duration-base ease-agent"
                  style={{ width: `${Math.round(activity.progress * 100)}%` }}
                />
              </div>
            )}
          </div>
        </div>
      </section>
    );
  }

  return (
    <section
      data-qa="spec-run-progress"
      aria-label="Live Spec progress"
      className={cn(
        "rounded-2xl border border-border-subtle bg-bg-secondary/80 shadow-sm",
        "p-5 sm:p-6",
      )}
    >
      <div className="flex items-start gap-3">
        <span className="mt-1.5 size-2 shrink-0 rounded-full bg-accent breathe" aria-hidden />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
            <h1 className="font-serif text-xl font-semibold tracking-[-0.02em] text-ink">
              Building your spec
            </h1>
            <span className="text-xs tabular-nums text-ink-faint">
              {calls.length > 0 ? `${completed} of ${calls.length} updates complete` : "Starting now"}
            </span>
          </div>
          <p data-qa="spec-live-activity" aria-live="polite" className="mt-1 text-sm font-medium text-ink-secondary">
            {activity.label}
          </p>
        </div>
      </div>

      {visibleCalls.length > 0 && (
        <ol className="mt-4 overflow-hidden rounded-xl border border-border-subtle bg-bg">
          {visibleCalls.map((call) => {
            const detail = stepDetail(call);
            const title = specProgressTitle(call);
            return (
              <li
                key={call.id}
                className="grid min-h-10 grid-cols-[1rem_minmax(0,1fr)] items-center gap-x-2.5 border-t border-border-subtle px-3 py-2 first:border-t-0"
              >
                <StatusIcon status={call.status} />
                <div className="min-w-0">
                  <p className={cn("truncate text-sm", call.status === "in_progress" ? "font-medium text-ink" : "text-ink-muted")}>
                    {title}
                  </p>
                  {detail && detail !== call.title && detail !== title && (
                    <p className="mt-0.5 truncate text-xs text-ink-faint">{detail}</p>
                  )}
                </div>
              </li>
            );
          })}
        </ol>
      )}

      {activeResearch && (
        <div className="mt-4 border-t border-border-subtle pt-3">
          <ResearchOutline progress={activeResearch.progress} />
        </div>
      )}

      {activity.progress !== undefined && (
        <div className="mt-4">
          <div className="mb-1.5 flex items-center justify-between text-xs text-ink-faint">
            <span>{activity.steps ? `${activity.steps.done} of ${activity.steps.total} planned steps` : "Overall progress"}</span>
            <span>{Math.round(activity.progress * 100)}%</span>
          </div>
          <div
            role="progressbar"
            aria-label="Spec progress"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(activity.progress * 100)}
            className="h-1.5 overflow-hidden rounded-full bg-bg-tertiary"
          >
            <div
              className="h-full rounded-full bg-accent transition-[width] duration-base ease-agent"
              style={{ width: `${Math.round(activity.progress * 100)}%` }}
            />
          </div>
        </div>
      )}
    </section>
  );
}
