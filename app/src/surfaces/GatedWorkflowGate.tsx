import { ArrowUpRight, LockKeyhole, Sparkles } from "lucide-react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import type { GatedWorkflow } from "../lib/slashCommands";
import type { ProductWorkflowAccessCopy } from "../product/productModule";
import { RISE_SMALL, accessibleMotion } from "../lib/motion";

export function GatedWorkflowGate({
  workflow,
  accessCopy,
  covered,
  checkingAccess,
  running,
  onRun,
  onViewAccess,
  onDismiss,
}: {
  workflow: GatedWorkflow;
  accessCopy?: ProductWorkflowAccessCopy;
  covered: boolean;
  checkingAccess: boolean;
  running: boolean;
  onRun: () => void;
  onViewAccess: () => void;
  onDismiss: () => void;
}) {
  const reduce = useReducedMotion();
  return (
    <m.div
      layout={!reduce}
      role="region"
      aria-live="polite"
      aria-label={`${workflow.label} access`}
      className="conversation-column-width mx-auto mb-2 w-full rounded-2xl border border-accent/20 bg-accent-subtle px-4 py-3 shadow-soft"
    >
      <div className="flex items-start gap-3">
        <span className="grid size-8 shrink-0 place-items-center rounded-xl bg-accent/12 text-accent">
          <LockKeyhole className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <p className="text-sm font-semibold text-ink">{workflow.label}</p>
            <span className="flex items-center gap-1 rounded-md bg-accent/10 px-1.5 py-0.5 text-xs font-medium text-accent">
              <Sparkles className="size-3" />
              {accessCopy?.badge ?? "Restricted workflow"}
            </span>
          </div>
          <p className="mt-1 text-sm leading-relaxed text-ink-secondary">
            {workflow.value}
          </p>
          <AnimatePresence mode="wait" initial={false}>
            <m.div
              key={covered ? "covered" : checkingAccess ? "checking" : "unavailable"}
              {...accessibleMotion(RISE_SMALL, reduce)}
            >
              <p className="mt-1 text-xs text-ink-muted">
                {covered
                  ? accessCopy?.available ?? "Access is ready. Run this saved request now."
                  : checkingAccess
                    ? accessCopy?.checking ?? "Checking access. Your request stays right here."
                    : accessCopy?.unavailable ?? "This workflow requires product access. Your request is saved."}
              </p>
              <div className="mt-3 flex flex-wrap items-center gap-2">
                {covered ? (
                  <button
                    type="button"
                    onClick={onRun}
                    disabled={running}
                    className="rounded-lg bg-accent px-3 py-1.5 text-sm font-semibold text-on-accent transition hover:bg-accent-hover disabled:opacity-60"
                  >
                    {running ? "Starting…" : "Run now"}
                  </button>
                ) : (
                  <button
                    type="button"
                    onClick={onViewAccess}
                    disabled={checkingAccess}
                    className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-semibold text-on-accent transition hover:bg-accent-hover disabled:opacity-60"
                  >
                    {checkingAccess ? "Checking access…" : accessCopy?.actionLabel ?? "Review access"}
                    {!checkingAccess && <ArrowUpRight className="size-3.5" />}
                  </button>
                )}
                <button
                  type="button"
                  onClick={onDismiss}
                  className="rounded-lg px-3 py-1.5 text-sm font-medium text-ink-muted transition hover:bg-bg-hover hover:text-ink"
                >
                  Keep editing
                </button>
              </div>
            </m.div>
          </AnimatePresence>
        </div>
      </div>
    </m.div>
  );
}
