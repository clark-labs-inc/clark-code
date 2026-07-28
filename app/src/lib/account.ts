// Account/billing bridge — Clark platform API key provisioning + billing state.
//
// The desktop never asks the user to paste an API key: after Google sign-in it
// mints a "Clark Code" key with the user's Clark JWT and stores it. Billing
// state powers the profile/subscription view. Both go through host-side Tauri
// commands (no WebView CORS) and are gated to the desktop app + a real token.

import { invoke } from "@tauri-apps/api/core";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import type { AuthSession } from "./auth";
import type { CloudCreds } from "./cloudHistory";

/** clarkchat.com billing/subscription page (where users buy credits + manage
 *  their plan). Same Google account → same Clark wallet as the desktop. */
export function clarkBillingUrl(): string {
  const origin = (
    (import.meta.env.VITE_CLARK_AUTH_ORIGIN as string | undefined) ??
    "https://www.clarkchat.com"
  ).replace(/\/+$/, "");
  return `${origin}/billing`;
}

/** Open a URL in the system browser (desktop) or a new tab (browser preview). */
export async function openExternal(url: string): Promise<void> {
  try {
    await shellOpen(url);
  } catch {
    if (typeof window !== "undefined") window.open(url, "_blank", "noopener");
  }
}

/** Mint a Clark Code platform API key; returns the full `ck_live_…` string. */
export function provisionCodeKey(c: CloudCreds): Promise<string> {
  return invoke<string>("clark_provision_code_key", {
    endpoint: c.endpoint,
    token: c.token,
  });
}

/** Account identity persisted beside the opaque Clark Code key. The stable
 * server user id is authoritative; normalized email only supports sessions
 * written by older desktop builds before that id was persisted. */
export function codeKeyAccountBinding(auth: AuthSession | null): string | null {
  const id = auth?.user.id?.trim();
  if (id) return `id:${id}`;
  const email = auth?.user.email?.trim().toLowerCase();
  return email ? `email:${email}` : null;
}

export function codeKeyMatchesAccount(
  apiKey: string,
  apiKeyOwner: string | undefined,
  auth: AuthSession | null,
): boolean {
  const owner = codeKeyAccountBinding(auth);
  return Boolean(apiKey.trim() && owner && apiKeyOwner === owner);
}

/** Subscription / plan / credit balance, as returned by `GET /api/billing/me`. */
export interface CreditAccount {
  available_credits: number;
  lifetime_granted: number;
  lifetime_spent: number;
  is_unlimited: boolean;
}

export interface Subscription {
  status: string; // active | trialing | past_due | canceled | …
  plan_key?: string | null;
  cancel_at_period_end?: boolean;
  current_period_end?: string | null;
  source_provider?: string | null;
}

/** A durable billing-ledger row. The server supplies this—never the client. */
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

export function billingMe(c: CloudCreds): Promise<BillingSummary> {
  return invoke<BillingSummary>("clark_billing_me", {
    endpoint: c.endpoint,
    token: c.token,
  });
}

/** The wallet Clark Code actually admits and debits for new runs. */
export function effectiveBilling(billing: BillingSummary | null): EffectiveBilling | null {
  if (!billing) return null;
  return billing.effective ?? {
    owner_kind: "user",
    display_name: "Personal",
    credits: billing.credits,
    subscription: billing.subscription,
    ledger: billing.ledger ?? [],
  };
}

export function effectiveBalance(billing: BillingSummary | null): {
  available_credits: number;
  is_unlimited: boolean;
} | null {
  const effective = effectiveBilling(billing);
  if (!effective) return null;
  return effective.balance ?? effective.credits ?? null;
}

/** Percentage of the current effective billing limit consumed by Clark Code.
 * Prefer workspace coverage when present, with the top-level personal value as
 * a rolling-upgrade fallback. */
export function effectiveUsagePercent(billing: BillingSummary | null): number | null {
  const percent = effectiveBilling(billing)?.credit_usage?.percent_used
    ?? billing?.credit_usage?.percent_used;
  if (typeof percent !== "number" || !Number.isFinite(percent)) return null;
  return Math.min(100, Math.max(0, percent));
}

export function billingPlanLabel(planKey?: string | null): string {
  if (!planKey) return "No active plan";
  return planKey
    .split("_")
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

/** The newest server-issued reward earned from completed paid activity. */
export function latestActivityReward(billing: BillingSummary | null): ActivityReward | null {
  const entry = (effectiveBilling(billing)?.ledger ?? []).find(
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

export type CreditState = "ok" | "low" | "out";

/** Warn below ~$2 of credits, hard-stop at zero. Unlimited / enforcement-off
 *  accounts never warn. */
const LOW_CREDIT_DOLLARS = 2;

export function creditState(billing: BillingSummary | null): CreditState {
  if (!billing || !billing.enforcement_enabled) return "ok";
  const c = effectiveBalance(billing);
  const access = effectiveBilling(billing)?.access_state;
  if (!c) return "ok";
  if (access === "unlimited" || c.is_unlimited) return "ok";
  if (access === "usage_limited" || c.available_credits <= 0) return "out";
  const perDollar = billing.credits_per_dollar;
  if (!perDollar || !Number.isFinite(perDollar)) return "ok";
  return c.available_credits < perDollar * LOW_CREDIT_DOLLARS ? "low" : "ok";
}

/** User-facing value for the effective account's usage row. Spendable access
 * wins over the cycle percentage: the latter is based on nominal period
 * allowance and can remain below 100 after the actual balance reaches zero. */
export function effectiveLimitLabel(billing: BillingSummary | null): string {
  if (creditState(billing) === "out") return "Out of credits";
  const effective = effectiveBilling(billing);
  const balance = effectiveBalance(billing);
  if (effective?.access_state === "unlimited" || balance?.is_unlimited) return "No limit";
  const percent = effectiveUsagePercent(billing);
  return percent === null ? "—" : `${percent}%`;
}
