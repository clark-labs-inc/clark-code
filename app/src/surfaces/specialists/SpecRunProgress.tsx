import type { ToolCall } from "../../core-bridge/types";
import type { Activity } from "../../lib/activity";

interface SpecRunProgressProps {
  activity: Activity;
  calls: readonly ToolCall[];
  compact?: boolean;
}

export function SpecRunProgress({ activity, calls, compact = false }: SpecRunProgressProps) {
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
      className="rounded-2xl border border-border-subtle bg-bg-secondary/80 p-5 shadow-sm sm:p-6"
    >
      <div className="flex items-start gap-3">
        <span className="mt-1.5 size-2 shrink-0 rounded-full bg-accent breathe" aria-hidden />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
            <h1 className="font-serif text-xl font-semibold tracking-[-0.02em] text-ink">
              Building your spec
            </h1>
          </div>
          <p data-qa="spec-live-activity" aria-live="polite" className="mt-1 text-sm font-medium text-ink-secondary">
            Writing the first draft…
          </p>
        </div>
      </div>

      {activity.progress !== undefined && (
        <div className="mt-4">
          <div className="mb-1.5 flex items-center justify-between text-xs text-ink-faint">
            <span>Draft progress</span>
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
