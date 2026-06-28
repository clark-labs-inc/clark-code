import { describe, expect, it } from "vitest";
import { creditState, type BillingSummary } from "./account";

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
          credits: { available_credits: 0, lifetime_granted: 0, lifetime_spent: 0, is_unlimited: true },
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
});
