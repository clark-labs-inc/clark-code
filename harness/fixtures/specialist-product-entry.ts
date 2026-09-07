import {
  installProductModule,
  neutralProduct,
} from "../../app/src/product/productModule";

const specialistCatalog = {
  schemaVersion: 1,
  catalogVersion: "1.0.0",
  catalogSha256: "1".repeat(64),
  trust: {
    source: "signed_app_bundle" as const,
    requiresSignedReleaseBinary: true,
  },
  manifests: [
    {
      kind: "security",
      version: "1.0.0",
      label: "Security",
      headline: "Find vulnerabilities you can prove",
      value: "Verified findings, safe PoCs, and remediation.",
      engine: "skill",
      entitlement: "subscription",
      modelPolicy: "specialist",
      defaultTab: "posture",
      defaultWorkflow: "security:security-scan",
      skillBindings: {
        "security:security-scan": "security:security-scan",
        "security:security-diff": "security:security-diff",
        "security:security-deep": "security:security-deep",
      },
      tabs: [
        { id: "posture", label: "Posture" },
        { id: "findings", label: "Findings" },
        { id: "zero-days", label: "Zero-day lab" },
        { id: "campaigns", label: "Campaigns" },
        { id: "scans", label: "Scans" },
      ],
      slashCommands: [
        {
          prefixes: ["/security-deep", "$security:security-deep"],
          tab: "scans",
          workflow: "security:security-deep",
          promptPrefix: "Run a deep security scan. ",
        },
        {
          prefixes: ["/security-diff", "$security:security-diff"],
          tab: "scans",
          workflow: "security:security-diff",
          promptPrefix: "Review the current diff for security regressions. ",
        },
        {
          prefixes: ["/security", "$security:security-scan"],
          tab: "posture",
          workflow: "security:security-scan",
        },
      ],
    },
  ],
};

installProductModule({
  ...neutralProduct,
  branding: { id: "specialist_e2e", name: "Clark Code", shortName: "Clark" },
  authRequired: true,
  specialistCatalog,
  localAgent: {
    ...neutralProduct.localAgent,
    providerExtra: ({ specialist }) => specialist?.kind === "security"
      ? {
          cloud_advisor: {
            organization_id: specialist.organizationId,
            specialist: specialist.kind,
            workflow: specialist.workflow,
            execution_residency: "local_only",
            training_consent: "explicit_user",
          },
        }
      : {},
  },
});
