import { describe, expect, it, vi } from "vitest";

describe("cloud specialist metadata", () => {
  it("keeps registered context and drops retired or malformed bindings", async () => {
    vi.resetModules();
    const { installProductModule, neutralProduct } = await import("../product/productModule");
    installProductModule({
      ...neutralProduct,
      specialistCatalog: {
        schemaVersion: 1,
        catalogVersion: "1.0.0",
        catalogSha256: "1".repeat(64),
        trust: {
          source: "signed_app_bundle",
          requiresSignedReleaseBinary: true,
        },
        manifests: [{
          kind: "security",
          version: "1.0.0",
          label: "Security",
          headline: "Find vulnerabilities you can prove",
          value: "Verified findings and remediation.",
          engine: "skill",
          entitlement: "subscription",
          modelPolicy: "specialist",
          tabs: [{ id: "posture", label: "Posture" }],
          defaultTab: "posture",
          defaultWorkflow: "security:security-scan",
          skillBindings: { "security:security-scan": "security:security-scan" },
          slashCommands: [],
        }],
      },
    });
    const { metaFromSummary } = await import("./cloudHistoryTypes");
    const summary = {
      id: "conversation-1",
      title: "Review",
      provider: "local",
      createdAt: 1,
      updatedAt: 2,
      rev: 3,
    };

    expect(metaFromSummary({
      ...summary,
      specialistContext: {
        kind: "security",
        workflow: "security:security-scan",
      },
    }).specialist).toEqual({
      kind: "security",
      workflow: "security:security-scan",
    });
    expect(metaFromSummary({
      ...summary,
      specialistContext: { kind: "retired", workflow: "retired:work" },
    }).specialist).toBeUndefined();
    expect(metaFromSummary({
      ...summary,
      specialistContext: { kind: "security", workflow: 42 },
    }).specialist).toBeUndefined();
  });
});
