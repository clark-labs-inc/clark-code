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
      defaultTab: "chat",
      defaultWorkflow: "security:assistant",
      skillBindings: {
        "security:assistant": "security:assistant",
        "security:security-scan": "security:security-scan",
        "security:security-diff": "security:security-diff",
        "security:security-deep": "security:security-deep",
      },
      tabs: [{ id: "chat", label: "Chat" }],
      slashCommands: [
        { prefixes: ["/security", "$security:assistant"], tab: "chat", workflow: "security:assistant" },
        {
          prefixes: ["/security-deep", "$security:security-deep"],
          tab: "chat",
          workflow: "security:security-deep",
          promptPrefix: "Run a deep security scan. ",
        },
        {
          prefixes: ["/security-diff", "$security:security-diff"],
          tab: "chat",
          workflow: "security:security-diff",
          promptPrefix: "Review the current diff for security regressions. ",
        },
        {
          prefixes: ["/security-scan", "$security:security-scan"],
          tab: "chat",
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
});
