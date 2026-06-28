import { CreditCard, ArrowUpRight } from "lucide-react";
import { clarkBillingUrl, openExternal } from "../lib/account";

/** Shown in the conversation when a run fails because the account is out of Clark
 *  credits — sends the user to clarkchat.com to add credits / pick a plan. */
export function UpgradePrompt() {
  return (
    <div className="rounded-xl border border-warning/30 bg-warning/10 px-4 py-3">
      <div className="flex items-start gap-3">
        <CreditCard className="mt-0.5 size-4 shrink-0 text-warning" />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium text-ink">You're out of Clark credits</p>
          <p className="mt-0.5 text-xs leading-relaxed text-ink-secondary">
            Add credits or choose a plan on clarkchat.com — sign in with the same Google
            account and your Clark Code session picks up right where it left off.
          </p>
        </div>
        <button
          onClick={() => void openExternal(clarkBillingUrl())}
          className="flex shrink-0 items-center gap-1.5 rounded-lg bg-ink px-3 py-1.5 text-xs font-semibold text-bg transition hover:bg-accent-hover"
        >
          Add credits
          <ArrowUpRight className="size-3.5" />
        </button>
      </div>
    </div>
  );
}
