import { useState } from "react";
import { CreditCard, ArrowUpRight, X } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import {
  creditState,
  creditDollars,
  clarkBillingUrl,
  openExternal,
  type CreditState,
} from "../lib/account";
import { cn } from "../lib/cn";

/** Proactive credit warning under the top bar: an amber, dismissible "running
 *  low" notice, escalating to a red, persistent "out of credits" bar. Both link
 *  to clarkchat.com to top up. (The in-conversation UpgradePrompt covers the
 *  moment a specific run is refused for credits.) */
export function CreditBanner() {
  const billing = useSessionStore((s) => s.billing);
  const [dismissed, setDismissed] = useState<CreditState | null>(null);
  const state = creditState(billing);

  if (state === "ok") return null;
  if (state === "low" && dismissed === "low") return null;

  const out = state === "out";
  const dollars = creditDollars(billing);

  return (
    <div
      className={cn(
        "flex items-center gap-3 border-b px-4 py-2 text-sm",
        out ? "border-danger/20 bg-danger/10" : "border-warning/20 bg-warning/10",
      )}
    >
      <CreditCard className={cn("size-4 shrink-0", out ? "text-danger" : "text-warning")} />
      <span className="min-w-0 flex-1 truncate text-ink">
        {out
          ? "You're out of Clark credits — add credits to keep coding."
          : `Running low on Clark credits — about $${dollars.toFixed(2)} left.`}
      </span>
      <button
        onClick={() => void openExternal(clarkBillingUrl())}
        className="flex min-h-8 shrink-0 items-center gap-1 rounded-lg bg-accent px-2.5 py-1 text-xs font-semibold text-on-accent transition duration-200 ease-clark hover:bg-accent-hover"
      >
        Add credits
        <ArrowUpRight className="size-3.5" />
      </button>
      {!out && (
        <button
          onClick={() => setDismissed("low")}
          aria-label="Dismiss"
          className="grid size-6 shrink-0 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink"
        >
          <X className="size-3.5" />
        </button>
      )}
    </div>
  );
}
