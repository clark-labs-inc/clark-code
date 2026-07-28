import { useState } from "react";
import { CreditCard, ArrowUpRight, X } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import {
  creditState,
  clarkBillingUrl,
  effectiveBilling,
  effectiveUsagePercent,
  openExternal,
  type BillingSummary,
  type CreditState,
} from "../lib/account";
import { cn } from "../lib/cn";
import {
  effectiveModelSettings,
  isIncludedCodingModel,
} from "../lib/localAgent";

export function creditBannerMessage(
  billing: BillingSummary | null,
  state: Exclude<CreditState, "ok">,
): string {
  const workspaceCovered = effectiveBilling(billing)?.owner_kind === "organization";
  const percent = effectiveUsagePercent(billing);
  if (state === "out") {
    return workspaceCovered
      ? "Clark Code is paused because the workspace has no available usage. Workspace billing needs attention."
      : "Clark Code is out of credits — review billing to keep coding.";
  }
  if (percent !== null) {
    return workspaceCovered
      ? `Your workspace has used ${percent}% of its Clark Code limit.`
      : `${percent}% of your Clark Code limit used.`;
  }
  return workspaceCovered
    ? "Your workspace is approaching its Clark Code usage limit."
    : "Approaching your Clark Code usage limit.";
}

/** Proactive credit warning under the top bar: an amber, dismissible "running
 *  low" notice, escalating to a red, persistent "out of credits" bar. Both link
 *  to clarkchat.com to top up. (The in-conversation UpgradePrompt covers the
 *  moment a specific run is refused for credits.) */
export function CreditBanner() {
  const billing = useSessionStore((s) => s.billing);
  const includedModel = useSessionStore((s) =>
    isIncludedCodingModel(
      effectiveModelSettings(s.localSettings, s.chatModels, s.session?.id ?? null).model,
    ),
  );
  const [dismissed, setDismissed] = useState<CreditState | null>(null);
  const state = creditState(billing);

  // The selected Free lane never performs paid billing admission or debits,
  // so a stale personal/workspace balance must not interrupt the composer.
  if (includedModel) return null;
  if (state === "ok") return null;
  if (state === "low" && dismissed === "low") return null;

  const out = state === "out";

  return (
    <div
      className={cn(
        "flex items-center gap-3 border-b px-4 py-2 text-sm",
        out ? "border-danger/20 bg-danger/10" : "border-warning/20 bg-warning/10",
      )}
    >
      <CreditCard className={cn("size-4 shrink-0", out ? "text-danger" : "text-warning")} />
      <span className="min-w-0 flex-1 truncate text-ink">
        {creditBannerMessage(billing, out ? "out" : "low")}
      </span>
      <button
        onClick={() => void openExternal(clarkBillingUrl())}
        className="flex min-h-8 shrink-0 items-center gap-1 rounded-lg bg-accent px-2.5 py-1 text-xs font-semibold text-on-accent transition duration-200 ease-clark hover:bg-accent-hover"
      >
        Review billing
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
