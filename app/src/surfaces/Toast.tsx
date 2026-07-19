import { useEffect, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { AlertTriangle, CheckCircle2, X } from "lucide-react";
import { DUR, EASE } from "../lib/motion";
import { useSessionStore } from "../store/sessionStore";
import { TEXT_SIZE_PERCENTAGES, type TextSize } from "../lib/useTextSize";

/** Brief browser-style feedback for the global text-size shortcuts. `signal`
 * increments for every shortcut press so the timeout also resets when the
 * current preset is already at its minimum or maximum. */
export function TextSizeToast({ textSize, signal }: { textSize: TextSize; signal: number }) {
  const [visible, setVisible] = useState(false);
  const reduce = useReducedMotion();

  useEffect(() => {
    if (signal === 0) return;
    setVisible(true);
    const timeout = setTimeout(() => setVisible(false), 1200);
    return () => clearTimeout(timeout);
  }, [signal]);

  return (
    <AnimatePresence>
      {visible && (
        <motion.div
          initial={reduce ? { opacity: 0 } : { opacity: 0, y: -4, scale: 0.96 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={reduce ? { opacity: 0 } : { opacity: 0, y: -4, scale: 0.96 }}
          transition={{ duration: DUR.fast, ease: EASE.out }}
          role="status"
          aria-live="polite"
          className="pointer-events-none fixed right-4 top-4 z-[90] rounded-lg border border-border-subtle bg-bg-elevated/95 px-3 py-1.5 font-mono text-sm tabular-nums text-ink shadow-lg backdrop-blur-sm"
        >
          {TEXT_SIZE_PERCENTAGES[textSize]}%
        </motion.div>
      )}
    </AnimatePresence>
  );
}

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
          transition={{ duration: DUR.base, ease: EASE.out }}
          role="status"
          className="fixed bottom-4 left-1/2 z-[90] flex max-w-[calc(100vw-2rem)] -translate-x-1/2 items-center gap-2.5 rounded-xl border border-border-subtle bg-bg-elevated px-3.5 py-2.5 shadow-lg"
        >
          <CheckCircle2 className="size-4 shrink-0 text-success" />
          <span className="min-w-0 text-sm text-ink">{notice}</span>
          <button
            onClick={dismiss}
            aria-label="Dismiss"
            className="grid size-8 shrink-0 place-items-center rounded-md text-ink-faint transition hover:bg-bg-hover hover:text-ink"
          >
            <X className="size-3.5" />
          </button>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

/** Non-fatal warning sibling of `NoticeToast`, backing the store's `warning`
 *  channel — e.g. a cloud-sync hiccup mid-run. Deliberately NOT the red error
 *  banner: the run it reports on is still alive. */
export function WarningToast() {
  const warning = useSessionStore((s) => s.warning);
  const dismiss = useSessionStore((s) => s.dismissWarning);
  const reduce = useReducedMotion();

  useEffect(() => {
    if (!warning) return;
    const t = setTimeout(dismiss, 8000);
    return () => clearTimeout(t);
  }, [warning, dismiss]);

  return (
    <AnimatePresence>
      {warning && (
        <motion.div
          key="warning"
          initial={reduce ? { opacity: 0 } : { opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          exit={reduce ? { opacity: 0 } : { opacity: 0, y: 12 }}
          transition={{ duration: DUR.base, ease: EASE.out }}
          role="status"
          className="fixed bottom-4 left-1/2 z-[90] flex max-w-[calc(100vw-2rem)] -translate-x-1/2 items-center gap-2.5 rounded-xl border border-border-subtle bg-bg-elevated px-3.5 py-2.5 shadow-lg"
        >
          <AlertTriangle className="size-4 shrink-0 text-warning" />
          <span className="min-w-0 text-sm text-ink">{warning}</span>
          <button
            onClick={dismiss}
            aria-label="Dismiss"
            className="grid size-8 shrink-0 place-items-center rounded-md text-ink-faint transition hover:bg-bg-hover hover:text-ink"
          >
            <X className="size-3.5" />
          </button>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
