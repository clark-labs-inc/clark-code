import { useEffect } from "react";
import { Gift, Sparkles, X } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { cn } from "../lib/cn";

function rewardCopy(tier: "base" | "bonus" | "jackpot", credits: number): { title: string; detail: string } {
  const amount = credits.toLocaleString();
  if (tier === "jackpot") {
    return { title: "Jackpot reward", detail: `Your work earned +${amount} credits.` };
  }
  if (tier === "bonus") {
    return { title: "Bonus reward", detail: `Your work earned +${amount} credits.` };
  }
  return { title: "Activity reward", detail: `Your work earned +${amount} credits.` };
}

/** A calm, dismissible receipt for a reward the billing ledger already issued.
 * It intentionally celebrates finished work instead of a persistent login. */
export function ActivityRewardToast() {
  const reward = useSessionStore((s) => s.activityReward);
  const dismiss = useSessionStore((s) => s.dismissActivityReward);

  useEffect(() => {
    if (!reward) return;
    const timer = window.setTimeout(dismiss, 9_000);
    return () => window.clearTimeout(timer);
  }, [dismiss, reward]);

  if (!reward) return null;
  const copy = rewardCopy(reward.tier, reward.credits);
  const special = reward.tier !== "base";

  return (
    <aside
      role="status"
      aria-live="polite"
      className={cn(
        "fixed right-4 top-4 z-50 flex w-[min(22rem,calc(100vw-2rem))] items-start gap-3 rounded-2xl border p-3.5 shadow-lifted backdrop-blur",
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
        onClick={dismiss}
        aria-label="Dismiss activity reward"
        className="grid size-7 shrink-0 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover hover:text-ink"
      >
        <X className="size-3.5" />
      </button>
    </aside>
  );
}
