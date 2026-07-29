import { useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Check, FilePlus2, ListChecks, MessageSquareMore, Play } from "lucide-react";
import { cn } from "../lib/cn";
import { useSessionStore } from "../store/sessionStore";
import { MD_CLASSES } from "./Message";
import type { ProposedPlan } from "../core-bridge/types";

export function ProposedPlanCard({ plan }: { plan: ProposedPlan }) {
  const decidePlan = useSessionStore((state) => state.decidePlan);
  const [feedbackOpen, setFeedbackOpen] = useState(false);
  const [feedback, setFeedback] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const awaiting = plan.status === "awaiting_decision";

  const decide = async (
    decision:
      | { action: "implement"; context: "current" | "fresh" }
      | { action: "continue_planning"; feedback?: string },
  ) => {
    setSubmitting(true);
    try {
      await decidePlan(plan.id, decision);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <section className="overflow-hidden rounded-xl border border-accent/25 bg-bg-elevated shadow-sm">
      <header className="flex items-center justify-between gap-3 border-b border-border-subtle px-4 py-3">
        <span className="flex items-center gap-2 text-sm font-semibold text-ink">
          <span className="grid size-7 place-items-center rounded-md bg-accent-subtle text-accent">
            {plan.status === "approved" ? <Check className="size-4" /> : <ListChecks className="size-4" />}
          </span>
          {plan.status === "approved" ? "Approved plan" : "Proposed plan"}
        </span>
        {plan.revision > 1 && <span className="text-xs text-ink-faint">Revision {plan.revision}</span>}
      </header>

      <div className={cn(MD_CLASSES, "px-4 py-3 text-sm")}>
        <Markdown remarkPlugins={[remarkGfm]}>{plan.markdown}</Markdown>
      </div>

      {awaiting && (
        <div className="border-t border-border-subtle px-3 py-3">
          {feedbackOpen ? (
            <div className="space-y-2">
              <textarea
                value={feedback}
                onChange={(event) => setFeedback(event.target.value)}
                placeholder="What should change in the plan?"
                rows={3}
                autoFocus
                className="w-full resize-y rounded-lg border border-border bg-bg-sunken px-3 py-2 text-sm text-ink outline-none focus:border-accent"
              />
              <div className="flex justify-end gap-2">
                <button type="button" onClick={() => setFeedbackOpen(false)} className="rounded-lg px-3 py-2 text-xs font-medium text-ink-muted hover:bg-bg-hover">
                  Cancel
                </button>
                <button
                  type="button"
                  disabled={submitting || !feedback.trim()}
                  onClick={() => void decide({ action: "continue_planning", feedback: feedback.trim() })}
                  className="rounded-lg bg-accent px-3 py-2 text-xs font-semibold text-on-accent disabled:opacity-40"
                >
                  Continue planning
                </button>
              </div>
            </div>
          ) : (
            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                disabled={submitting}
                onClick={() => void decide({ action: "implement", context: "current" })}
                className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-2 text-xs font-semibold text-on-accent disabled:opacity-40"
              >
                <Play className="size-3.5" /> Implement
              </button>
              <button
                type="button"
                disabled={submitting}
                onClick={() => void decide({ action: "implement", context: "fresh" })}
                className="flex items-center gap-1.5 rounded-lg border border-border px-3 py-2 text-xs font-medium text-ink-secondary hover:bg-bg-hover disabled:opacity-40"
              >
                <FilePlus2 className="size-3.5" /> Fresh context
              </button>
              <button
                type="button"
                disabled={submitting}
                onClick={() => setFeedbackOpen(true)}
                className="flex items-center gap-1.5 rounded-lg px-3 py-2 text-xs font-medium text-ink-secondary hover:bg-bg-hover disabled:opacity-40"
              >
                <MessageSquareMore className="size-3.5" /> Keep planning
              </button>
            </div>
          )}
        </div>
      )}
    </section>
  );
}
