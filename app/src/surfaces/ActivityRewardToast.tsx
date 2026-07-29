import { useEffect } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { Gift, Sparkles, X } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { cn } from "../lib/cn";
import { DUR, EASE } from "../lib/motion";
import type { ActivityReward } from "../lib/account";

function rewardCopy(tier: "base" | "bonus" | "jackpot"): { title: string; detail: string } {
  if (tier === "jackpot") {
    return { title: "Jackpot reward", detail: "Your work earned an activity reward." };
  }
  if (tier === "bonus") {
    return { title: "Bonus reward", detail: "Your work earned an activity reward." };
  }
  return { title: "Activity reward", detail: "Your work earned an activity reward." };
}

/** A calm, dismissible receipt for a reward the billing ledger already issued.
 * It intentionally celebrates finished work instead of a persistent login. */
export function ActivityRewardToast() {
  const reward = useSessionStore((s) => s.activityReward);
  const dismiss = useSessionStore((s) => s.dismissActivityReward);
  const reduce = useReducedMotion() ?? false;

  useEffect(() => {
    if (!reward) return;
    const timer = window.setTimeout(dismiss, 9_000);
    return () => window.clearTimeout(timer);
  }, [dismiss, reward]);

  return (
    <AnimatePresence>
      {reward && <ActivityRewardReceipt reward={reward} onDismiss={dismiss} reduceMotion={reduce} />}
    </AnimatePresence>
  );
}

export function ActivityRewardReceipt({
  reward,
  onDismiss,
  reduceMotion = false,
}: {
  reward: ActivityReward;
  onDismiss: () => void;
  reduceMotion?: boolean;
}) {
  const copy = rewardCopy(reward.tier);
  const special = reward.tier !== "base";

  return (
    <motion.div
      initial={reduceMotion ? false : { opacity: 0, y: -8, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={reduceMotion ? undefined : { opacity: 0, y: -8, scale: 0.98 }}
      transition={{ duration: reduceMotion ? 0 : DUR.fast, ease: EASE.out }}
      className="fixed right-4 top-4 z-50 w-[min(22rem,calc(100vw-2rem))]"
    >
      <aside
        role="status"
        aria-live="polite"
        className={cn(
          "popover-surface flex w-full items-start gap-3 rounded-2xl border p-3.5 shadow-lifted backdrop-blur",
          special ? "border-warning/30 bg-warning/10" : "border-success/25 bg-bg-elevated/95",
        )}
      >
        <span
          className={cn(
            "grid size-9 shrink-0 place-items-center rounded-xl",
            special ? "bg-warning/20 text-warning" : "bg-success/15 text-success",
          )}
        >
          {special ? <Sparkles className="size-4.5" /> : <Gift className="size-4.5" />}
        </span>
        <div className="min-w-0 flex-1 pt-0.5">
          <div className="text-sm font-semibold text-ink">{copy.title}</div>
          <div className="mt-0.5 text-sm text-ink-secondary">{copy.detail}</div>
        </div>
        <button
          type="button"
          onClick={onDismiss}
          aria-label="Dismiss activity reward"
          className="grid size-7 shrink-0 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover hover:text-ink"
        >
          <X className="size-3.5" />
        </button>
      </aside>
    </motion.div>
  );
}
