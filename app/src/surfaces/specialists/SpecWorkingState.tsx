import { FileText } from "lucide-react";

import type { ToolCall } from "../../core-bridge/types";
import type { Activity } from "../../lib/activity";
import { SpecRunProgress } from "./SpecRunProgress";

interface SpecWorkingStateProps {
  activity: Activity;
  calls: readonly ToolCall[];
  hasSubmittedPrompt: boolean;
  documentUnavailable?: boolean;
}

export function SpecWorkingState({
  activity,
  calls,
  hasSubmittedPrompt,
  documentUnavailable = false,
}: SpecWorkingStateProps) {
  if (documentUnavailable) {
    return (
      <div role="alert" className="mx-auto flex min-h-[22rem] max-w-[32rem] flex-col items-center justify-center text-center">
        <div className="grid size-11 place-items-center rounded-xl bg-warning/10 text-warning">
          <FileText className="size-5" />
        </div>
        <h1 className="mt-4 font-serif text-2xl font-semibold tracking-[-0.025em] text-ink">
          This spec couldn’t be opened
        </h1>
        <p className="mt-2 max-w-[28rem] text-sm leading-6 text-ink-muted">
          The saved document was not replaced. Reopen the spec or ask Clark to try reading it again.
        </p>
      </div>
    );
  }

  if (!activity.busy) {
    return (
      <div className="mx-auto flex min-h-[22rem] max-w-[32rem] flex-col items-center justify-center text-center">
        <div className="grid size-11 place-items-center rounded-xl bg-accent-subtle text-accent">
          <FileText className="size-5" />
        </div>
        <h1 className="mt-4 font-serif text-2xl font-semibold tracking-[-0.025em] text-ink">
          Start with the change you want to make
        </h1>
        <p className="mt-2 max-w-[28rem] text-sm leading-6 text-ink-muted">
          Describe the idea in your own words. Your spec will grow from the decisions you make—not from a pre-filled template.
        </p>
      </div>
    );
  }

  return (
    <div
      role="status"
      aria-live="polite"
      aria-label={`${hasSubmittedPrompt ? "Building your spec" : "Getting your spec ready"}: ${activity.label}`}
      className="mx-auto flex min-h-[22rem] max-w-[36rem] flex-col justify-center"
    >
      <SpecRunProgress activity={activity} calls={calls} />
      <p className="mt-4 text-center text-xs leading-5 text-ink-faint">
        The first real draft will appear here as soon as it is ready. You can stop the run at any time.
      </p>
    </div>
  );
}
