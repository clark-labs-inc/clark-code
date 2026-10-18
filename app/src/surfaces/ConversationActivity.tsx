import { memo } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { FADE, accessibleMotion } from "../lib/motion";

/** Status changes share one line of tail space, including the idle state.
 * Predicting an answer's height made narration/tool handoffs resize the whole
 * bottom-pinned transcript. Prose now owns only its actual rendered height. */
export const ConversationActivity = memo(function ConversationActivity({
  show,
  label,
  detail,
}: {
  show: boolean;
  label: string;
  detail?: string;
}) {
  const reduce = useReducedMotion();
  return (
    <div className="h-6 shrink-0" data-qa="conversation-activity" aria-hidden={!show}>
      <AnimatePresence initial={false}>
        {show && (
          <m.div {...accessibleMotion(FADE, reduce)} key="activity" className="flex h-6 items-center gap-2.5 text-sm text-ink-muted">
            <span className="activity-dots flex shrink-0 items-center gap-[3px]" aria-hidden>
              <span className="size-1.5 rounded-full bg-accent" />
              <span className="size-1.5 rounded-full bg-accent" />
              <span className="size-1.5 rounded-full bg-accent" />
            </span>
            <span className="truncate">
              {label || "Thinking…"}
              {detail && <span className="ml-1.5 font-mono text-xs text-ink-faint">{detail}</span>}
            </span>
          </m.div>
        )}
      </AnimatePresence>
    </div>
  );
});
