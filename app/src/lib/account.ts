// Account/billing bridge — Clark platform API key provisioning + billing state.
//
// The desktop never asks the user to paste an API key: after Google sign-in it
// mints a "Clark Code" key with the user's Clark JWT and stores it. Billing
// state powers the profile/subscription view. Both go through host-side Tauri
// commands (no WebView CORS) and are gated to the desktop app + a real token.

import { invoke } from "@tauri-apps/api/core";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
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

export interface BillingSummary {
  stripe_enabled: boolean;
  enforcement_enabled: boolean;
  credits_per_dollar: number;
  credits: CreditAccount;
  subscription?: Subscription | null;
  plans?: unknown[];
  ledger?: unknown[];
}

export function billingMe(c: CloudCreds): Promise<BillingSummary> {
  return invoke<BillingSummary>("clark_billing_me", {
    endpoint: c.endpoint,
    token: c.token,
  });
}

export type CreditState = "ok" | "low" | "out";

/** Warn below ~$2 of credits, hard-stop at zero. Unlimited / enforcement-off
 *  accounts never warn. */
const LOW_CREDIT_DOLLARS = 2;

export function creditState(billing: BillingSummary | null): CreditState {
  if (!billing || !billing.enforcement_enabled) return "ok";
  const c = billing.credits;
  if (c.is_unlimited) return "ok";
  if (c.available_credits <= 0) return "out";
  const perDollar = Math.max(1, billing.credits_per_dollar);
  return c.available_credits < perDollar * LOW_CREDIT_DOLLARS ? "low" : "ok";
}

/** Approximate remaining dollar value of the credit balance, for display. */
export function creditDollars(billing: BillingSummary | null): number {
  if (!billing) return 0;
  return billing.credits.available_credits / Math.max(1, billing.credits_per_dollar);
}
