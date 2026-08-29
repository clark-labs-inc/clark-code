import { describe, expect, it } from "vitest";
import {
  specialistNeedsEntitlementVerification,
  specialistReadRoots,
} from "./specialists";

describe("specialistReadRoots", () => {
  it("preserves Scout's account-scoped census roots", () => {
    expect(specialistReadRoots(
      { kind: "scout" },
      ["/repos/payments", "/repos/identity"],
    )).toEqual(["/repos/payments", "/repos/identity"]);
  });

  it("grants no filesystem roots to unregistered lenses", () => {
    expect(specialistReadRoots({ kind: "rsi" }, ["/repos/recent"])).toEqual([]);
  });
});

describe("specialist entitlement verification", () => {
  it("does not send included specialists through the paid entitlement boundary", () => {
    expect(specialistNeedsEntitlementVerification("included")).toBe(false);
    expect(specialistNeedsEntitlementVerification("subscription")).toBe(true);
  });
});
