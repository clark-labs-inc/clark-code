import { describe, expect, it } from "vitest";
import {
  projectClarkCodeBilling,
  type BillingSummary,
  type EffectiveBilling,
} from "./billing";

function summary(effective: EffectiveBilling): BillingSummary {
  return {
    stripe_enabled: true,
    enforcement_enabled: true,
    credits_per_dollar: 100,
    effective,
  };
}

const activePersonal: EffectiveBilling = {
  owner_kind: "user",
  display_name: "Personal",
  coverage_status: "ready",
  access_state: "ready",
  products: ["clark_web", "clark_code"],
  balance: { available_credits: 10_000, is_unlimited: false },
  subscription: { status: "active", plan_key: "scale" },
};

describe("Clark Code billing policy", () => {
  it.each([
    {
      name: "ready subscription",
      effective: activePersonal,
      state: "ready",
      reason: "subscription_ready",
      canRun: true,
    },
    {
      name: "coverage action required",
      effective: { ...activePersonal, coverage_status: "action_needed" as const },
      state: "action_needed",
      reason: "coverage_action_needed",
      canRun: false,
    },
    {
      name: "coverage unavailable",
      effective: { ...activePersonal, coverage_status: "unavailable" as const },
      state: "action_needed",
      reason: "coverage_unavailable",
      canRun: false,
    },
    {
      name: "usage exhausted",
      effective: { ...activePersonal, access_state: "usage_limited" as const },
      state: "action_needed",
      reason: "usage_limited",
      canRun: false,
    },
    {
      name: "past due",
      effective: { ...activePersonal, subscription: { status: "past_due" } },
      state: "action_needed",
      reason: "past_due",
      canRun: false,
    },
    {
      name: "product excluded",
      effective: { ...activePersonal, products: ["clark_web" as const] },
      state: "not_included",
      reason: "product_not_included",
      canRun: false,
    },
    {
      name: "workspace seat not assigned",
      effective: {
        ...activePersonal,
        owner_kind: "organization" as const,
        seat: { purchased: 2, assigned: 1, assigned_to_current_user: false },
      },
      state: "not_included",
      reason: "workspace_seat_unassigned",
      canRun: false,
    },
    {
      name: "inactive subscription",
      effective: { ...activePersonal, subscription: { status: "canceled" } },
      state: "not_included",
      reason: "subscription_inactive",
      canRun: false,
    },
  ])("classifies $name once for every consumer", ({ effective, state, reason, canRun }) => {
    const policy = projectClarkCodeBilling(summary(effective));
    expect(policy.coverage).toEqual({
      state,
      reason,
      canRunSubscriberWorkflows: canRun,
    });
  });

  it("keeps a missing snapshot unknown instead of guessing Free or paid", () => {
    expect(projectClarkCodeBilling(null).coverage).toEqual({
      state: "unknown",
      reason: "missing_snapshot",
      canRunSubscriberWorkflows: false,
    });
  });

  it("derives tier, account status, usage, and recovery from the same decision", () => {
    const exhausted = projectClarkCodeBilling(summary({
      ...activePersonal,
      access_state: "usage_limited",
      balance: { available_credits: 0, is_unlimited: false },
    }));
    expect(exhausted).toMatchObject({
      tier: "action_needed",
      accountStatus: "action_needed",
      billingFailureResolved: false,
      usage: { state: "out", limitLabel: "Out of credits" },
    });

    const ready = projectClarkCodeBilling(summary(activePersonal));
    expect(ready).toMatchObject({
      tier: "paid",
      accountStatus: "ready",
      billingFailureResolved: true,
      usage: { state: "ok" },
    });

    const inconsistentReady = projectClarkCodeBilling(summary({
      ...activePersonal,
      balance: { available_credits: 0, is_unlimited: false },
    }));
    expect(inconsistentReady.coverage).toEqual({
      state: "action_needed",
      reason: "balance_exhausted",
      canRunSubscriberWorkflows: false,
    });
  });
});
