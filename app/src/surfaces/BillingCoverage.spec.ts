import { describe, expect, it } from "vitest";
import { creditState, type BillingSummary } from "../lib/account";
import { isIncludedCodingModel } from "../lib/localAgent";
import { creditBannerMessage } from "./CreditBanner";
import { upgradePromptCopy } from "./UpgradePrompt";

function workspaceBilling(available: number): BillingSummary {
  return {
    stripe_enabled: true,
    enforcement_enabled: true,
    credits_per_dollar: 100,
    credits: {
      available_credits: 0,
      lifetime_granted: 0,
      lifetime_spent: 0,
      is_unlimited: false,
    },
    effective: {
      owner_kind: "organization",
      display_name: "clarkslabs.com",
      access_state: available > 0 ? "ready" : "usage_limited",
      credit_usage: { percent_used: available > 0 ? 25 : 100 },
      coverage_status: available > 0 ? "ready" : "action_needed",
      products: ["clark_web", "clark_code"],
      balance: { available_credits: available, is_unlimited: false },
      plan: { plan_key: "team_monthly", name: "Team Monthly" },
      seat: { purchased: 1, assigned: 1, assigned_to_current_user: true },
      credits: {
        available_credits: available,
        lifetime_granted: 8_521,
        lifetime_spent: 944,
        is_unlimited: false,
      },
      subscription: { status: "active", plan_key: "team_monthly" },
      ledger: [],
    },
  };
}

describe("Clark Code billing coverage copy", () => {
  it("does not tell a workspace-covered member to buy personal credits", () => {
    const billing = workspaceBilling(0);
    const prompt = upgradePromptCopy(billing);
    const banner = creditBannerMessage(billing, "out");

    expect(prompt.title).toContain("Workspace billing needs attention");
    expect(prompt.detail).toContain("no Clark Code usage available");
    expect(prompt.detail).toContain("assigned workspace seat");
    expect(prompt.detail).not.toContain("choose a plan");
    expect(banner).toContain("workspace has no available usage");
    expect(banner).toContain("Workspace billing needs attention");
    expect(`${prompt.detail} ${banner}`).not.toContain("credits");
  });

  it("uses the effective workspace balance instead of the empty personal wallet", () => {
    expect(creditState(workspaceBilling(7_577))).toBe("ok");
  });

  it("treats only the managed Free alias as included", () => {
    expect(isIncludedCodingModel("clark-code:free")).toBe(true);
    expect(isIncludedCodingModel("qwen/qwen3.7-flash")).toBe(false);
    expect(isIncludedCodingModel("clark-code")).toBe(false);
  });
});
