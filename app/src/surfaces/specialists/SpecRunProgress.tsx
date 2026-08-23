import { useState } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { ChevronDown } from "lucide-react";
import { cn } from "../../lib/cn";
import { DUR, EASE, REDUCED_EXIT } from "../../lib/motion";
import { specRunReceipt, type SpecLabelSource, type SpecLiveStatus } from "../../lib/specProgress";
import { SpecToolList, SpecToolTrail } from "./SpecToolTrail";
import type { ToolCall } from "../../core-bridge/types";
import type { Activity } from "../../lib/activity";

interface SpecRunProgressProps {
  status: SpecLiveStatus;
  activity: Activity;
  calls: readonly ToolCall[];
  compact?: boolean;
}

/** The visible label changes on every streamed token, which a screen reader must
 *  not follow. Announce one coarse sentence per rung instead. */
const ANNOUNCEMENT: Record<SpecLabelSource, string> = {
  tool_progress: "Running a tool.",
  tool_stream: "Running a tool.",
  tool_title: "Running a tool.",
  checklist: "Working through the plan.",
  drafting: "Writing the spec.",
  commentary: "Explaining the change.",
  thinking: "Thinking.",
  last_receipt: "Finished a step.",
  starting: "Getting set up.",
  unknown: "Working.",
};

function ProgressBar({ progress, thin }: { progress: number; thin?: boolean }) {
  const percent = Math.round(progress * 100);
  return (
    <div
      role="progressbar"
      aria-label="Spec progress"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent}
      className={cn("overflow-hidden rounded-full bg-bg-tertiary", thin ? "h-1" : "h-1.5")}
    >
      <div
        className="h-full rounded-full bg-accent transition-[width] duration-base ease-agent"
        style={{ width: `${percent}%` }}
      />
    </div>
  );
}

/** Live label + streamed detail. Kept to one truncated line so changing copy
 *  never reflows the card while tokens arrive. */
function LiveLabel({ status, className }: { status: SpecLiveStatus; className?: string }) {
  return (
    <span aria-hidden className={cn("min-w-0 flex-1 truncate text-left", className)}>
      {status.label}
      {status.detail && (
        <span className="ml-1.5 font-mono text-xs text-ink-faint">{status.detail}</span>
      )}
    </span>
  );
}

export function SpecRunProgress({ status, activity, calls, compact = false }: SpecRunProgressProps) {
  const [open, setOpen] = useState(false);
  const reduce = useReducedMotion();
  const receipt = specRunReceipt(calls, activity.steps);
  const canOpen = calls.length > 0;
  // The bar is a picture of the plan position; skip it when the receipt already
  // spells that out, and keep it when the receipt is about changed files instead.
  const bar = activity.progress !== undefined && receipt?.kind !== "steps"
    ? activity.progress
    : undefined;

  const announcement = (
    <p data-qa="spec-live-activity" className="sr-only" aria-live="polite" aria-atomic="true">
      {ANNOUNCEMENT[status.source]}
    </p>
  );

  const detail = canOpen && (
    <AnimatePresence initial={false}>
      {open && (
        <m.div
          initial={reduce ? { opacity: 0 } : { height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          exit={reduce ? REDUCED_EXIT : { height: 0, opacity: 0 }}
          transition={{ duration: DUR.fast, ease: reduce ? EASE.out : EASE.inOut }}
          className="overflow-hidden"
        >
          {/* Bounded: a long turn must not push the document itself off screen. */}
          <div className="mt-3 max-h-64 overflow-y-auto border-t border-border-subtle pt-2.5 pr-1">
            <SpecToolList calls={calls} />
          </div>
        </m.div>
      )}
    </AnimatePresence>
  );

  if (compact) {
    return (
      <section
        data-qa="spec-run-progress"
        aria-label="Live Spec progress"
        className="mx-auto mb-5 max-w-[44rem] rounded-xl border border-border-subtle bg-bg-secondary/80 px-4 py-3 shadow-sm"
      >
        {announcement}
        <button
          type="button"
          onClick={() => canOpen && setOpen((value) => !value)}
          disabled={!canOpen}
          aria-expanded={open}
          className={cn(
            "flex w-full min-w-0 items-center gap-3 rounded-md text-sm font-medium text-ink",
            canOpen && "cursor-pointer",
          )}
        >
          <span className="size-2 shrink-0 rounded-full bg-accent breathe" aria-hidden />
          <LiveLabel status={status} />
          {receipt && (
            <span className="shrink-0 text-xs font-normal tabular-nums text-ink-faint">
              {receipt.text}
            </span>
          )}
          {canOpen && (
            <ChevronDown
              aria-hidden
              className={cn("size-3.5 shrink-0 text-ink-faint transition", open && "rotate-180")}
            />
          )}
        </button>

        {canOpen && (
          <div className="mt-2 pl-5">
            <SpecToolTrail calls={calls} />
          </div>
        )}

        {bar !== undefined && (
          <div className="mt-2">
            <ProgressBar progress={bar} thin />
          </div>
        )}

        {detail}
      </section>
    );
  }

  return (
    <section
      data-qa="spec-run-progress"
      aria-label="Live Spec progress"
      className="rounded-2xl border border-border-subtle bg-bg-secondary/80 p-5 shadow-sm sm:p-6"
    >
      {announcement}
      <div className="flex items-start gap-3">
        <span className="mt-1.5 size-2 shrink-0 rounded-full bg-accent breathe" aria-hidden />
        <div className="min-w-0 flex-1">
          <h1 className="font-serif text-xl font-semibold tracking-[-0.02em] text-ink">
            Building your spec
          </h1>
          <p className="mt-1 flex min-w-0 text-sm font-medium text-ink-secondary">
            <LiveLabel status={status} />
          </p>
          {canOpen && (
            <div className="mt-3">
              <SpecToolTrail calls={calls} />
            </div>
          )}
        </div>
      </div>

      {activity.progress !== undefined && (
        <div className="mt-4">
          <div className="mb-1.5 flex items-center justify-between text-xs text-ink-faint">
            <span>Draft progress</span>
            <span>{Math.round(activity.progress * 100)}%</span>
          </div>
          <ProgressBar progress={activity.progress} />
        </div>
      )}

      {canOpen && (
        <button
          type="button"
          onClick={() => setOpen((value) => !value)}
          aria-expanded={open}
          className="mt-3 flex items-center gap-1.5 text-xs font-medium text-ink-muted transition hover:text-ink-secondary"
        >
          {open ? "Hide steps" : "Show steps"}
          <ChevronDown aria-hidden className={cn("size-3.5 transition", open && "rotate-180")} />
        </button>
      )}
      {detail}
    </section>
  );
}
