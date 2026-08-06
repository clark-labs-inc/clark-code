import { describe, expect, it } from "vitest";
import {
  authAccountMatches,
  codeKeyAccountBinding,
} from "./account";
import {
  describeBillingTransition,
  latestActivityReward,
  projectClarkCodeBilling,
  type BillingSummary,
} from "./billing";

describe("Clark Code key ownership", () => {
  const auth = {
    user: { id: "user-1", name: "Stan", email: "STAN@example.com", method: "google" as const },
  };

  it("binds native credential partitions to the stable Clark user id", () => {
    expect(codeKeyAccountBinding(auth)).toBe("id:user-1");
  });

  it("keeps descriptor refreshes but rejects a different stable account", () => {
    expect(authAccountMatches(auth, { ...auth })).toBe(true);
    expect(authAccountMatches(auth, {
      ...auth,
      user: { ...auth.user, id: "user-2" },
    })).toBe(false);
  });
});

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

describe("Clark Code billing projection", () => {
  it("ok with plenty / unlimited / enforcement off / no data", () => {
    expect(projectClarkCodeBilling(null).usage.state).toBe("ok");
    expect(projectClarkCodeBilling(billing(1000)).usage.state).toBe("ok"); // $10 > $2
    expect(
      projectClarkCodeBilling(
        billing(0, {
          effective: {
            owner_kind: "user",
            display_name: "Personal",
            credits: { available_credits: 0, lifetime_granted: 0, lifetime_spent: 0, is_unlimited: true },
            subscription: null,
            ledger: [],
          },
        }),
      ).usage.state,
    ).toBe("ok");
    expect(projectClarkCodeBilling(billing(0, { enforcement_enabled: false })).usage.state).toBe("ok");
  });

  it("low under ~$2 of credits", () => {
    expect(projectClarkCodeBilling(billing(150)).usage.state).toBe("low"); // $1.50
    expect(projectClarkCodeBilling(billing(1)).usage.state).toBe("low");
  });

  it("out at zero or below", () => {
    expect(projectClarkCodeBilling(billing(0)).usage.state).toBe("out");
    expect(projectClarkCodeBilling(billing(-10)).usage.state).toBe("out");
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

    expect(projectClarkCodeBilling(summary).usage.state).toBe("ok");
  });

  it("falls back to personal billing during a rolling server upgrade", () => {
    expect(projectClarkCodeBilling(billing(1_000, { effective: undefined })).usage.state).toBe("ok");
  });

  it("prefers the effective workspace percentage and clamps it for display", () => {
    const summary = billing(10_000, {
      credit_usage: { percent_used: 12 },
      effective: {
        owner_kind: "organization",
        display_name: "Clark Labs",
        access_state: "ready",
        credit_usage: { percent_used: 140 },
      },
    });

    expect(projectClarkCodeBilling(summary).usage.percentUsed).toBe(100);
  });

  it("falls back to the top-level personal percentage", () => {
    expect(projectClarkCodeBilling(billing(10_000, {
      credit_usage: { percent_used: 42 },
      effective: undefined,
    })).usage.percentUsed).toBe(42);
    expect(projectClarkCodeBilling(null).usage.percentUsed).toBeNull();
  });

  it("shows exhausted spendable credits instead of a stale cycle percentage", () => {
    const summary = billing(0, {
      effective: {
        owner_kind: "organization",
        display_name: "Clark Labs",
        access_state: "usage_limited",
        credit_usage: { percent_used: 21 },
        coverage_status: "action_needed",
        balance: { available_credits: 0, is_unlimited: false },
      },
    });

    expect(projectClarkCodeBilling(summary).usage.percentUsed).toBe(21);
    expect(projectClarkCodeBilling(summary).usage.limitLabel).toBe("Out of credits");
  });

  it("keeps percentage and unlimited labels for accounts that can run", () => {
    expect(projectClarkCodeBilling(billing(10_000, {
      credit_usage: { percent_used: 42 },
      effective: undefined,
    })).usage.limitLabel).toBe("42%");
    expect(projectClarkCodeBilling(billing(0, {
      effective: {
        owner_kind: "user",
        display_name: "Personal",
        access_state: "unlimited",
        balance: { available_credits: 0, is_unlimited: true },
      },
    })).usage.limitLabel).toBe("No limit");
  });
});

describe("billing plan presentation", () => {
  it("renders machine plan keys as product copy", () => {
    expect(projectClarkCodeBilling(billing(500, {
      effective: {
        owner_kind: "user",
        display_name: "Personal",
        subscription: { status: "active", plan_key: "team_monthly" },
      },
    })).planLabel).toBe("Team Monthly");
    expect(projectClarkCodeBilling(null).planLabel).toBe("No active plan");
  });
});

describe("Clark Code subscription coverage", () => {
  it("accepts active personal plans and an assigned subscribed workspace seat", () => {
    expect(projectClarkCodeBilling(billing(500, {
      effective: {
        owner_kind: "user",
        display_name: "Personal",
        subscription: { status: "active", plan_key: "scale" },
      },
    })).coverage.canRunSubscriberWorkflows).toBe(true);
    expect(projectClarkCodeBilling(billing(500, {
      effective: {
        owner_kind: "organization",
        display_name: "Clark Labs",
        coverage_status: "ready",
        subscription: { status: "active", plan_key: "team_monthly" },
        seat: { purchased: 3, assigned: 2, assigned_to_current_user: true },
      },
    })).coverage.canRunSubscriberWorkflows).toBe(true);
  });

  it("does not treat credits alone or unusable coverage as a subscription", () => {
    expect(projectClarkCodeBilling(billing(500)).coverage.canRunSubscriberWorkflows).toBe(false);
    expect(projectClarkCodeBilling(billing(500, {
      effective: {
        owner_kind: "organization",
        display_name: "Clark Labs",
        coverage_status: "action_needed",
        subscription: { status: "past_due", plan_key: "team_monthly" },
      },
    })).coverage.canRunSubscriberWorkflows).toBe(false);
    expect(projectClarkCodeBilling(billing(500, {
      effective: {
        owner_kind: "organization",
        display_name: "Clark Labs",
        coverage_status: "action_needed",
        subscription: { status: "active", plan_key: "team_monthly" },
        seat: { purchased: 3, assigned: 2, assigned_to_current_user: true },
      },
    })).coverage.canRunSubscriberWorkflows).toBe(false);
    expect(projectClarkCodeBilling(billing(500, {
      effective: {
        owner_kind: "user",
        display_name: "Personal",
        coverage_status: "unavailable",
        subscription: { status: "active", plan_key: "scale" },
      },
    })).coverage.canRunSubscriberWorkflows).toBe(false);
    expect(projectClarkCodeBilling(billing(500, {
      effective: {
        owner_kind: "user",
        display_name: "Personal",
        coverage_status: "ready",
        access_state: "usage_limited",
        products: ["clark_web"],
        subscription: { status: "active", plan_key: "scale" },
      },
    })).coverage.canRunSubscriberWorkflows).toBe(false);
  });
});

describe("billing tier transitions", () => {
  it("does not report an active tier when Clark Code coverage is unusable", () => {
    expect(projectClarkCodeBilling(billing(500, {
      effective: {
        owner_kind: "organization",
        display_name: "Clark Labs",
        coverage_status: "unavailable",
        subscription: { status: "active", plan_key: "team_monthly" },
      },
    })).tier).toBe("action_needed");
    expect(projectClarkCodeBilling(billing(500, {
      effective: {
        owner_kind: "user",
        display_name: "Personal",
        coverage_status: "ready",
        access_state: "usage_limited",
        subscription: { status: "active", plan_key: "scale" },
      },
    })).tier).toBe("action_needed");
  });

  it("announces upgrades without announcing the first account load", () => {
    const free = billing(500);
    const paid = billing(500, {
      effective: {
        owner_kind: "user",
        display_name: "Personal",
        subscription: { status: "active", plan_key: "scale" },
      },
    });
    expect(describeBillingTransition(null, paid)).toBeNull();
    expect(describeBillingTransition(free, paid, 7)).toMatchObject({
      id: 7,
      kind: "upgraded",
      tier: "paid",
      title: "Your Clark subscription is ready",
    });
  });

  it("distinguishes workspace, action-needed, and Free transitions", () => {
    const workspace = billing(500, {
      effective: {
        owner_kind: "organization",
        display_name: "Clark Labs",
        coverage_status: "ready",
        subscription: { status: "active", plan_key: "team_monthly" },
        seat: { purchased: 3, assigned: 2, assigned_to_current_user: true },
      },
    });
    const attention = billing(500, {
      effective: {
        owner_kind: "organization",
        display_name: "Clark Labs",
        coverage_status: "action_needed",
      },
    });
    expect(projectClarkCodeBilling(workspace).tier).toBe("workspace");
    expect(describeBillingTransition(workspace, attention)?.kind).toBe("attention");
    expect(describeBillingTransition(attention, billing(500))?.kind).toBe("downgraded");
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
    expect(projectClarkCodeBilling(summary).usage).toMatchObject({
      availableCredits: 12_345,
      isUnlimited: false,
      state: "ok",
    });
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
