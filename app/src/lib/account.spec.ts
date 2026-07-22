import { describe, expect, it } from "vitest";
import {
  billingPlanLabel,
  creditState,
  effectiveBalance,
  latestActivityReward,
  type BillingSummary,
} from "./account";

function billing(available: number, over: Partial<BillingSummary> = {}): BillingSummary {
  return {
    stripe_enabled: true,
    enforcement_enabled: true,
    credits_per_dollar: 100,
    credits: {
      available_credits: available,
      lifetime_granted: 0,
      lifetime_spent: 0,
      is_unlimited: false,
    },
    effective: {
      owner_kind: "user",
      display_name: "Personal",
      credits: {
        available_credits: available,
        lifetime_granted: 0,
        lifetime_spent: 0,
        is_unlimited: false,
      },
      subscription: null,
      ledger: [],
    },
    ...over,
  };
}

describe("creditState", () => {
  it("ok with plenty / unlimited / enforcement off / no data", () => {
    expect(creditState(null)).toBe("ok");
    expect(creditState(billing(1000))).toBe("ok"); // $10 > $2
    expect(
      creditState(
        billing(0, {
          effective: {
            owner_kind: "user",
            display_name: "Personal",
            credits: { available_credits: 0, lifetime_granted: 0, lifetime_spent: 0, is_unlimited: true },
            subscription: null,
            ledger: [],
          },
        }),
      ),
    ).toBe("ok");
    expect(creditState(billing(0, { enforcement_enabled: false }))).toBe("ok");
  });

  it("low under ~$2 of credits", () => {
    expect(creditState(billing(150))).toBe("low"); // $1.50
    expect(creditState(billing(1))).toBe("low");
  });

  it("out at zero or below", () => {
    expect(creditState(billing(0))).toBe("out");
    expect(creditState(billing(-10))).toBe("out");
  });

  it("uses the effective team wallet instead of an empty personal wallet", () => {
    const summary = billing(0, {
      effective: {
        owner_kind: "organization",
        display_name: "Clark Labs",
        domain: "clarkslabs.com",
        access_state: "ready",
        coverage_status: "ready",
        balance: { available_credits: 7_985, is_unlimited: false },
        plan: { plan_key: "team_monthly", name: "Team Monthly" },
        seat: { purchased: 1, assigned: 1, assigned_to_current_user: true },
        subscription: { status: "active", plan_key: "team_monthly" },
        ledger: [],
      },
    });

    expect(creditState(summary)).toBe("ok");
  });

  it("falls back to personal billing during a rolling server upgrade", () => {
    expect(creditState(billing(1_000, { effective: undefined }))).toBe("ok");
  });
});

describe("billingPlanLabel", () => {
  it("renders machine plan keys as product copy", () => {
    expect(billingPlanLabel("team_monthly")).toBe("Team Monthly");
    expect(billingPlanLabel(null)).toBe("No active plan");
  });
});

describe("latestActivityReward", () => {
  it("accepts the current production envelope without legacy ledger fields", () => {
    const summary: BillingSummary = {
      stripe_enabled: true,
      enforcement_enabled: true,
      access_state: "ready",
      credit_usage: { percent_used: 0 },
      subscription: { status: "active", plan_key: "scale" },
      plans: [],
      effective: {
        owner_kind: "user",
        display_name: "Personal",
        access_state: "ready",
        credit_usage: { percent_used: 0 },
        coverage_status: "ready",
        products: ["clark_web", "clark_code"],
        balance: { available_credits: 12_345, is_unlimited: false },
        plan: { plan_key: "scale", name: "Scale" },
        subscription: { status: "active", plan_key: "scale" },
      },
      personal_fallback: {
        status: "active",
        access_state: "ready",
        balance: { available_credits: 12_345, is_unlimited: false },
        subscription: { status: "active", plan_key: "scale" },
      },
    };

    expect(latestActivityReward(summary)).toBeNull();
    expect(effectiveBalance(summary)).toEqual({ available_credits: 12_345, is_unlimited: false });
    expect(creditState(summary)).toBe("ok");
  });

  it("returns only a server-authored positive activity grant", () => {
    const summary = billing(500, {
      effective: {
        owner_kind: "user",
        display_name: "Personal",
        credits: { available_credits: 500, lifetime_granted: 0, lifetime_spent: 0, is_unlimited: false },
        subscription: null,
        ledger: [
          {
            id: "usage", amount: -15, direction: -1, reason: "run_usage", source_type: "runtime_run",
            source_id: "run-1", created_at: "2026-07-19T12:00:00Z",
          },
          {
            id: "reward", amount: 450, direction: 1, reason: "activity_reward", source_type: "billable_activity",
            source_id: "run-1", reward_tier: "bonus", created_at: "2026-07-19T12:01:00Z",
          },
        ],
      },
    });

    expect(latestActivityReward(summary)).toEqual({
      id: "reward", credits: 450, tier: "bonus", createdAt: "2026-07-19T12:01:00Z",
    });
  });

  it("ignores old ledger rows that are not activity rewards", () => {
    expect(
      latestActivityReward(
        billing(500, {
          effective: {
            owner_kind: "user",
            display_name: "Personal",
            credits: { available_credits: 500, lifetime_granted: 0, lifetime_spent: 0, is_unlimited: false },
            subscription: null,
            ledger: [
              {
                id: "login", amount: 200, direction: 1, reason: "daily_login_grant", source_type: "daily_login",
                source_id: "old", created_at: "2026-07-18T12:00:00Z",
              },
            ],
          },
        }),
      ),
    ).toBeNull();
  });
});
