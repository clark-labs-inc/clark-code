import { describe, expect, it } from "vitest";
import {
  parseSpecialistCatalog,
  specialistAccessAfterProductFailure,
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

  it("turns a failed unknown product check into a retryable offline state", () => {
    expect(specialistAccessAfterProductFailure("loading", true)).toBe("offline");
    expect(specialistAccessAfterProductFailure("signed_out", true)).toBe("signed_out");
    expect(specialistAccessAfterProductFailure("ready", true)).toBe("ready");
  });
});

describe("specialist catalog authority", () => {
  it("rejects a manifest without a foundation presentation adapter", () => {
    expect(() => parseSpecialistCatalog({
      schemaVersion: 1,
      catalogVersion: "1.0.0",
      catalogSha256: "1".repeat(64),
      trust: {
        source: "signed_app_bundle",
        requiresSignedReleaseBinary: true,
      },
      manifests: [{
        kind: "retired",
        version: "1.0.0",
        label: "Retired",
        headline: "Unavailable specialist",
        value: "This manifest has no renderer adapter.",
        engine: "skill",
        entitlement: "subscription",
        modelPolicy: "specialist",
        tabs: [{ id: "work", label: "Work" }],
        defaultTab: "work",
        defaultWorkflow: "retired:work",
        skillBindings: { "retired:work": "retired:work" },
        slashCommands: [],
      }],
    })).toThrow("no registered presentation adapter");
  });
});
