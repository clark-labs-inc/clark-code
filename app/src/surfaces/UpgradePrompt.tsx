import { CreditCard, ArrowUpRight } from "lucide-react";
import {
  clarkBillingUrl,
  effectiveBilling,
  effectiveUsagePercent,
  openExternal,
  type BillingSummary,
} from "../lib/account";
import { useSessionStore } from "../store/sessionStore";

export function upgradePromptCopy(billing: BillingSummary | null): {
  title: string;
  detail: string;
} {
  if (effectiveBilling(billing)?.owner_kind === "organization") {
    const percent = effectiveUsagePercent(billing);
    return {
      title: "Workspace billing needs attention",
      detail: `${percent !== null ? `${percent}% of the workspace limit used. ` : ""}Clark Code uses your assigned workspace seat. Ask a workspace owner to review its billing, then retry this run.`,
    };
  }
  const percent = effectiveUsagePercent(billing);
  return {
    title: "Clark Code usage limit reached",
    detail: `${percent !== null ? `${percent}% of the current limit used. ` : ""}Review your Clark billing account, then retry this run. Your work stays here.`,
  };
}

/** Shown in the conversation when a run fails because the account is out of Clark
 *  credits — sends the user to clarkchat.com to add credits / pick a plan. */
export function UpgradePrompt() {
  const billing = useSessionStore((s) => s.billing);
  const copy = upgradePromptCopy(billing);
  return (
    <div className="rounded-xl border border-warning/30 bg-warning/10 px-4 py-3">
      <div className="flex items-start gap-3">
        <CreditCard className="mt-0.5 size-4 shrink-0 text-warning" />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium text-ink">
            {copy.title}
          </p>
          <p className="mt-0.5 text-xs leading-relaxed text-ink-secondary">
            {copy.detail}
          </p>
        </div>
        <button
          onClick={() => void openExternal(clarkBillingUrl())}
          className="flex min-h-8 shrink-0 items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-on-accent transition duration-200 ease-clark hover:bg-accent-hover"
        >
          Review billing
          <ArrowUpRight className="size-3.5" />
        </button>
      </div>
    </div>
  );
}
