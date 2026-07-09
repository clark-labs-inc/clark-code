import { useEffect } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { CheckCircle2, X } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";

const EASE = [0.4, 0, 0.2, 1] as const;

/** Transient success/info confirmation, bottom-center, auto-dismissing. Backs
 *  the store's `notice` channel — used for user actions whose only other signal
 *  is a native OS notification, which is suppressed while the window is focused
 *  (e.g. "Share link copied"). Without this, those actions look like no-ops. */
export function NoticeToast() {
  const notice = useSessionStore((s) => s.notice);
  const dismiss = useSessionStore((s) => s.dismissNotice);
  const reduce = useReducedMotion();

  useEffect(() => {
    if (!notice) return;
    const t = setTimeout(dismiss, 4000);
    return () => clearTimeout(t);
  }, [notice, dismiss]);

  return (
    <AnimatePresence>
      {notice && (
        <motion.div
          key="notice"
          initial={reduce ? { opacity: 0 } : { opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          exit={reduce ? { opacity: 0 } : { opacity: 0, y: 12 }}
          transition={{ duration: 0.25, ease: EASE }}
          role="status"
          className="fixed bottom-4 left-1/2 z-[90] flex max-w-[calc(100vw-2rem)] -translate-x-1/2 items-center gap-2.5 rounded-xl border border-border-subtle bg-bg-elevated px-3.5 py-2.5 shadow-lg"
        >
          <CheckCircle2 className="size-4 shrink-0 text-success" />
          <span className="min-w-0 text-sm text-ink">{notice}</span>
          <button
            onClick={dismiss}
            aria-label="Dismiss"
            className="grid size-6 shrink-0 place-items-center rounded-md text-ink-faint transition hover:bg-bg-hover hover:text-ink"
          >
            <X className="size-3.5" />
          </button>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
