import { invoke } from "@tauri-apps/api/core";
import type { CloudCreds } from "./cloudHistory";

export interface CreditAccount {
  available_credits: number;
  lifetime_granted: number;
  lifetime_spent: number;
  is_unlimited: boolean;
}

export interface Subscription {
  status: string;
  plan_key?: string | null;
  cancel_at_period_end?: boolean;
  current_period_end?: string | null;
  source_provider?: string | null;
}

export interface LedgerEntry {
  id: string;
  amount: number;
  direction: -1 | 1;
  reason: string;
  source_type: string;
  source_id: string;
  reward_tier?: "base" | "bonus" | "jackpot";
  created_at: string;
}

export interface ActivityReward {
  id: string;
  credits: number;
  tier: "base" | "bonus" | "jackpot";
  createdAt: string;
}

export interface EffectiveBilling {
  owner_kind: "user" | "organization";
  display_name: string;
  domain?: string;
  access_state?: "ready" | "usage_limited" | "unlimited";
  credit_usage?: { percent_used: number };
  coverage_status?: "ready" | "action_needed" | "unavailable";
  products?: Array<"clark_web" | "clark_code">;
  balance?: { available_credits: number; is_unlimited: boolean };
  plan?: {
    plan_key: string;
    name: string;
    price_cents?: number | null;
    currency?: string | null;
    zero_decimal_currency?: boolean | null;
    billing_interval?: string | null;
    is_seat_based?: boolean | null;
  } | null;
  seat?: {
    purchased: number;
    assigned: number;
    assigned_to_current_user?: boolean | null;
  } | null;
  /** Legacy server field retained only during rolling upgrades. */
  credits?: CreditAccount;
  subscription?: Subscription | null;
  /** Legacy activity detail; current billing responses intentionally omit it. */
  ledger?: LedgerEntry[];
}

export interface BillingSummary {
  stripe_enabled: boolean;
  enforcement_enabled: boolean;
  access_state?: "ready" | "usage_limited" | "unlimited";
  credit_usage?: { percent_used: number };
  /** Legacy fields retained for compatibility with older Clark deployments. */
  credits_per_dollar?: number;
  credits?: CreditAccount;
  subscription?: Subscription | null;
  plans?: unknown[];
  ledger?: LedgerEntry[];
  payment_history?: unknown[];
  effective?: EffectiveBilling;
  personal_fallback?: {
    status: "active" | "inactive_workspace_coverage" | "unavailable";
    access_state: "ready" | "usage_limited" | "unlimited";
    balance: { available_credits: number; is_unlimited: boolean };
    subscription?: Subscription | null;
  };
}

export type ClarkCodeCoverageState = "unknown" | "ready" | "not_included" | "action_needed";
export type ClarkCodeCoverageReason =
  | "missing_snapshot"
  | "subscription_ready"
  | "coverage_action_needed"
  | "coverage_unavailable"
  | "usage_limited"
  | "balance_exhausted"
  | "past_due"
  | "product_not_included"
  | "workspace_seat_unassigned"
  | "subscription_inactive";
export type CreditState = "ok" | "low" | "out";
export type BillingTier = "free" | "trial" | "paid" | "workspace" | "action_needed";
export type BillingAccountStatus =
  | "ready"
  | "action_needed"
  | "active"
  | "trial"
  | "past_due"
  | "canceled"
  | "no_plan";

export interface ClarkCodeBillingState {
  effective: EffectiveBilling | null;
  ownerKind: "user" | "organization" | null;
  coverage: {
    state: ClarkCodeCoverageState;
    reason: ClarkCodeCoverageReason;
    canRunSubscriberWorkflows: boolean;
  };
  usage: {
    state: CreditState;
    percentUsed: number | null;
    availableCredits: number | null;
    isUnlimited: boolean;
    limitLabel: string;
  };
  tier: BillingTier;
  accountStatus: BillingAccountStatus;
  planLabel: string;
  billingFailureResolved: boolean;
}

export interface BillingTransition {
  id: number;
  kind: "upgraded" | "downgraded" | "changed" | "attention";
  title: string;
  detail: string;
  tier: BillingTier;
}

const ACTIVE_SUBSCRIPTIONS = new Set(["active", "trialing", "in_grace_period"]);
const LOW_CREDIT_DOLLARS = 2;

export function billingMe(_c: CloudCreds): Promise<BillingSummary> {
  return invoke<BillingSummary>("clark_billing_me");
}

function effectiveBilling(billing: BillingSummary | null): EffectiveBilling | null {
  if (!billing) return null;
  return billing.effective ?? {
    owner_kind: "user",
    display_name: "Personal",
    credits: billing.credits,
    subscription: billing.subscription,
    ledger: billing.ledger ?? [],
  };
}

function effectiveBalance(effective: EffectiveBilling | null): {
  available_credits: number;
  is_unlimited: boolean;
} | null {
  return effective?.balance ?? effective?.credits ?? null;
}

function usagePercent(billing: BillingSummary | null, effective: EffectiveBilling | null): number | null {
  const percent = effective?.credit_usage?.percent_used ?? billing?.credit_usage?.percent_used;
  if (typeof percent !== "number" || !Number.isFinite(percent)) return null;
  return Math.min(100, Math.max(0, percent));
}

function coverageDecision(effective: EffectiveBilling | null): ClarkCodeBillingState["coverage"] {
  if (!effective) {
    return { state: "unknown", reason: "missing_snapshot", canRunSubscriberWorkflows: false };
  }
  const status = effective.subscription?.status.toLowerCase() ?? "";
  if (effective.coverage_status === "action_needed") {
    return { state: "action_needed", reason: "coverage_action_needed", canRunSubscriberWorkflows: false };
  }
  if (effective.coverage_status === "unavailable") {
    return { state: "action_needed", reason: "coverage_unavailable", canRunSubscriberWorkflows: false };
  }
  if (effective.access_state === "usage_limited") {
    return { state: "action_needed", reason: "usage_limited", canRunSubscriberWorkflows: false };
  }
  if (status === "past_due") {
    return { state: "action_needed", reason: "past_due", canRunSubscriberWorkflows: false };
  }
  if (effective.products && !effective.products.includes("clark_code")) {
    return { state: "not_included", reason: "product_not_included", canRunSubscriberWorkflows: false };
  }
  if (
    effective.owner_kind === "organization"
    && effective.seat?.assigned_to_current_user === false
  ) {
    return { state: "not_included", reason: "workspace_seat_unassigned", canRunSubscriberWorkflows: false };
  }
  if (!ACTIVE_SUBSCRIPTIONS.has(status)) {
    return { state: "not_included", reason: "subscription_inactive", canRunSubscriberWorkflows: false };
  }
  return { state: "ready", reason: "subscription_ready", canRunSubscriberWorkflows: true };
}

function usageDecision(
  billing: BillingSummary | null,
  effective: EffectiveBilling | null,
): ClarkCodeBillingState["usage"] {
  const balance = effectiveBalance(effective);
  const percentUsed = usagePercent(billing, effective);
  const isUnlimited = effective?.access_state === "unlimited" || balance?.is_unlimited === true;
  let state: CreditState = "ok";
  if (billing?.enforcement_enabled && !isUnlimited && balance) {
    if (effective?.access_state === "usage_limited" || balance.available_credits <= 0) {
      state = "out";
    } else if (
      billing.credits_per_dollar
      && Number.isFinite(billing.credits_per_dollar)
      && balance.available_credits < billing.credits_per_dollar * LOW_CREDIT_DOLLARS
    ) {
      state = "low";
    }
  }
  const limitLabel = state === "out"
    ? "Out of credits"
    : isUnlimited
      ? "No limit"
      : percentUsed === null
        ? "—"
        : `${percentUsed}%`;
  return {
    state,
    percentUsed,
    availableCredits: balance?.available_credits ?? null,
    isUnlimited,
    limitLabel,
  };
}

function billingPlanLabel(planKey?: string | null): string {
  if (!planKey) return "No active plan";
  return planKey
    .split("_")
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

export function projectClarkCodeBilling(billing: BillingSummary | null): ClarkCodeBillingState {
  const effective = effectiveBilling(billing);
  const usage = usageDecision(billing, effective);
  const projectedCoverage = coverageDecision(effective);
  const coverage: ClarkCodeBillingState["coverage"] =
    projectedCoverage.state === "ready" && usage.state === "out"
      ? {
        state: "action_needed",
        reason: "balance_exhausted",
        canRunSubscriberWorkflows: false,
      }
      : projectedCoverage;
  const subscriptionStatus = effective?.subscription?.status.toLowerCase() ?? "";
  const needsAction = coverage.state === "action_needed" || usage.state === "out";
  const tier: BillingTier = needsAction
    ? "action_needed"
    : coverage.state !== "ready"
      ? "free"
      : effective?.owner_kind === "organization"
        ? "workspace"
        : subscriptionStatus === "trialing"
          ? "trial"
          : "paid";
  const accountStatus: BillingAccountStatus = needsAction
    ? "action_needed"
    : coverage.state === "ready"
      ? "ready"
      : subscriptionStatus === "active" || subscriptionStatus === "in_grace_period"
        ? "active"
        : subscriptionStatus === "trialing"
          ? "trial"
          : subscriptionStatus === "past_due"
            ? "past_due"
            : subscriptionStatus === "canceled"
              ? "canceled"
              : "no_plan";
  const planLabel = effective?.plan?.name
    ?? (effective?.owner_kind === "organization"
      ? "Workspace coverage"
      : billingPlanLabel(effective?.subscription?.plan_key));
  return {
    effective,
    ownerKind: effective?.owner_kind ?? null,
    coverage,
    usage,
    tier,
    accountStatus,
    planLabel,
    billingFailureResolved: billing !== null && !needsAction,
  };
}

export function billingAccountStatusPresentation(
  status: BillingAccountStatus,
): { label: string; tone: string } {
  switch (status) {
    case "ready": return { label: "Ready", tone: "text-success" };
    case "action_needed": return { label: "Action needed", tone: "text-warning" };
    case "active": return { label: "Active", tone: "text-success" };
    case "trial": return { label: "Trial", tone: "text-info" };
    case "past_due": return { label: "Past due", tone: "text-warning" };
    case "canceled": return { label: "Canceled", tone: "text-ink-muted" };
    case "no_plan": return { label: "No plan", tone: "text-ink-muted" };
  }
}

export function describeBillingTransition(
  previous: BillingSummary | null,
  next: BillingSummary,
  id: number = Date.now(),
): BillingTransition | null {
  if (!previous) return null;
  const from = projectClarkCodeBilling(previous).tier;
  const tier = projectClarkCodeBilling(next).tier;
  if (from === tier) return null;
  if (tier === "workspace") {
    return {
      id,
      kind: "upgraded",
      title: "Workspace coverage is ready",
      detail: "Scout, Security, and subscriber workflows are now available.",
      tier,
    };
  }
  if (tier === "trial" || tier === "paid") {
    return {
      id,
      kind: "upgraded",
      title: tier === "trial" ? "Your Clark trial is ready" : "Your Clark subscription is ready",
      detail: "Scout, Security, and subscriber workflows are now available.",
      tier,
    };
  }
  if (tier === "action_needed") {
    return {
      id,
      kind: "attention",
      title: "Clark coverage needs attention",
      detail: "Scout and Security are paused. Your specialist chats and drafts are safe.",
      tier,
    };
  }
  return {
    id,
    kind: "downgraded",
    title: "Free is now active",
    detail: "Scout and Security are paused. Clark Code, your specialist chats, and drafts remain safe.",
    tier,
  };
}

export function latestActivityReward(billing: BillingSummary | null): ActivityReward | null {
  const entry = (projectClarkCodeBilling(billing).effective?.ledger ?? []).find(
    (value) => value.reason === "activity_reward" && value.direction === 1 && value.amount > 0,
  );
  if (!entry) return null;
  return {
    id: entry.id,
    credits: entry.amount,
    tier: entry.reward_tier ?? "base",
    createdAt: entry.created_at,
  };
}
