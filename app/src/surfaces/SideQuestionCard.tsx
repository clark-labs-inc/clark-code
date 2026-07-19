// `/btw` side-question overlay — a dismissable modal that renders the answer
// to a forked, tool-less model call over the session transcript. It never
// interrupts or cancels the active run: dismissing only clears this overlay
// and drops any in-flight answer (the run is a separate run id, untouched).

import { useEffect, useRef } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { X } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { Md } from "./Message";
import { DUR, EASE } from "../lib/motion";

/** A short spinner — three pulsing dots, matching the conversation pending row. */
function Spinner() {
  return (
    <span className="flex items-center gap-[3px]" aria-hidden>
      {[0, 1, 2].map((i) => (
        <motion.span
          key={i}
          className="size-1.5 rounded-full bg-accent"
          animate={{ opacity: [0.3, 1, 0.3] }}
          transition={{ duration: 1.1, repeat: Infinity, delay: i * 0.18 }}
        />
      ))}
    </span>
  );
}

export function SideQuestionCard() {
  const reduce = useReducedMotion();
  const sideQuestion = useSessionStore((s) => s.sideQuestion);
  const dismiss = useSessionStore((s) => s.dismissSideQuestion);
  const cardRef = useRef<HTMLDivElement>(null);

  // Esc isolation: while the overlay is open, swallow Escape at the document
  // level BEFORE the composer's `onKey` can see it. The composer currently
  // routes empty-composer-Esc to `cancelActive()` (stops the main run); this
  // listener stops propagation so Esc can never cancel the run from here. It's
  // installed only while the overlay is mounted, and removed on unmount.
  useEffect(() => {
    if (!sideQuestion) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      dismiss();
    };
    // Capture phase: runs before React's synthetic (bubble) handlers, so the
    // composer's onKeyDown never receives the event.
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, [sideQuestion, dismiss]);

  // Auto-focus the card on open so screen readers announce it and keyboard
  // users can Tab to the Close button immediately.
  useEffect(() => {
    if (sideQuestion) cardRef.current?.focus();
  }, [sideQuestion]);

  return (
    <AnimatePresence>
      {sideQuestion && (
        <motion.div
          className="fixed inset-0 z-50 flex items-start justify-center bg-black/40 px-4 pt-[12vh]"
          initial={reduce ? false : { opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={reduce ? { opacity: 0, transition: { duration: 0 } } : { opacity: 0 }}
          transition={{ duration: reduce ? 0 : DUR.fast, ease: EASE.out }}
          onMouseDown={(e) => e.target === e.currentTarget && dismiss()}
        >
          <motion.div
            ref={cardRef}
            tabIndex={-1}
            role="dialog"
            aria-modal="true"
            aria-label="Side question"
            initial={reduce ? false : { opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={reduce ? { opacity: 0, transition: { duration: 0 } } : { opacity: 0, y: 8 }}
            transition={{ duration: reduce ? 0 : DUR.fast, ease: EASE.out }}
            className="popover-surface flex max-h-[70vh] w-full max-w-xl flex-col overflow-hidden rounded-[22px] bg-bg-elevated shadow-lifted ring-1 ring-border-subtle outline-none"
          >
            {/* Header: `/btw` accent + the question + a visible Close button. */}
            <div className="flex items-start gap-3 border-b border-border-subtle px-5 py-3.5">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 text-sm">
                  <span className="font-semibold text-accent">/btw</span>
                  <span className="truncate text-ink-muted">{sideQuestion.question}</span>
                </div>
              </div>
              <button
                onClick={dismiss}
                aria-label="Close side question"
                className="-mr-1 grid size-7 shrink-0 place-items-center rounded-md text-ink-faint transition hover:bg-bg-hover hover:text-ink"
              >
                <X className="size-4" />
              </button>
            </div>

            {/* Body: the answer (markdown), a spinner while loading, or an
                error. Scrolls independently of the transcript behind it. */}
            <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4 text-sm leading-relaxed text-ink-secondary">
              {sideQuestion.error ? (
                <p className="text-danger">{sideQuestion.error}</p>
              ) : sideQuestion.answer != null ? (
                <Md>{sideQuestion.answer}</Md>
              ) : (
                <div className="flex items-center gap-2.5 text-ink-muted">
                  <Spinner />
                  <span>Answering…</span>
                </div>
              )}
            </div>

            {/* Footer hint: tells the user how to close it and that the run is
                unaffected. Kept short to avoid clutter. */}
            <div className="flex items-center justify-between gap-3 border-t border-border-subtle px-5 py-2.5 text-xs text-ink-faint">
              <span>The run keeps going in the background.</span>
              <span>
                Press <kbd className="font-sans font-medium text-ink-muted">Esc</kbd> to close
              </span>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
