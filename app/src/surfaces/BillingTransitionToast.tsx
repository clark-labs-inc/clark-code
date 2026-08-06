import { useEffect } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { ArrowUpRight, BadgeCheck, CircleAlert, Sparkles, X } from "lucide-react";
import type { BillingTransition } from "../lib/billing";
import { clarkBillingUrl, openExternal } from "../lib/account";
import {
  DUR,
  FADE,
  accessibleMotion,
  staggeredTransition,
} from "../lib/motion";
import { useSessionStore } from "../store/sessionStore";

export function BillingTransitionReceipt({
  transition,
  onDismiss,
  onViewBilling,
}: {
  transition: BillingTransition;
  onDismiss: () => void;
  onViewBilling: () => void;
}) {
  const positive = transition.kind === "upgraded";
  const needsAction = transition.kind === "attention";
  const Icon = positive ? BadgeCheck : needsAction ? CircleAlert : Sparkles;
  return (
    <div className="flex max-w-md items-start gap-3 rounded-2xl border border-border-subtle bg-bg-elevated px-4 py-3 shadow-lifted">
      <span className={`grid size-9 shrink-0 place-items-center rounded-xl ${
        positive ? "bg-success/12 text-success" : "bg-accent-subtle text-accent"
      }`}>
        <Icon className="size-4.5" />
      </span>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-semibold text-ink">{transition.title}</p>
        <p className="mt-0.5 text-xs leading-relaxed text-ink-muted">{transition.detail}</p>
        {!positive && (
          <button
            type="button"
            onClick={onViewBilling}
            className="mt-2 flex items-center gap-1 text-xs font-semibold text-accent hover:text-accent-hover"
          >
            Review billing <ArrowUpRight className="size-3" />
          </button>
        )}
      </div>
      <button
        type="button"
        onClick={onDismiss}
        aria-label="Dismiss subscription update"
        className="grid size-7 shrink-0 place-items-center rounded-md text-ink-faint transition hover:bg-bg-hover hover:text-ink"
      >
        <X className="size-3.5" />
      </button>
    </div>
  );
}

export function BillingTransitionToast() {
  const transition = useSessionStore((state) => state.billingTransition);
  const dismiss = useSessionStore((state) => state.dismissBillingTransition);
  const reduce = useReducedMotion();

  useEffect(() => {
    if (!transition) return;
    const timer = window.setTimeout(dismiss, 8_000);
    return () => window.clearTimeout(timer);
  }, [dismiss, transition]);

  return (
    <AnimatePresence initial={false}>
      {transition && (
        <m.div
          key={transition.id}
          role="status"
          aria-live="polite"
          {...accessibleMotion(FADE, reduce)}
          transition={staggeredTransition(reduce, 0, 0.04, { duration: DUR.base })}
          className="fixed bottom-4 left-1/2 z-[92] -translate-x-1/2 px-4"
        >
          <BillingTransitionReceipt
            transition={transition}
            onDismiss={dismiss}
            onViewBilling={() => void openExternal(clarkBillingUrl())}
          />
        </m.div>
      )}
    </AnimatePresence>
  );
}
