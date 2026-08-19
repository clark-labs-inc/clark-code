import {
  installProductModule,
  neutralProduct,
  type ProductInitialSpecialistDocument,
} from "../../app/src/product/productModule";

interface PreparationReceipt {
  conversationId: string;
  filename: string;
  initialDocument: ProductInitialSpecialistDocument | null;
}

export const SPECIALIST_MATRIX_PRODUCT_PROBE =
  "agent-desktop:specialist-matrix-product-probe";

const filenames = new Map<string, string>();

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
      kind: "spec",
      version: "1.0.0",
      label: "Spec",
      headline: "Turn a feature idea into a complete specification",
      value: "A living feature document shaped through plain-language conversation.",
      engine: "skill",
      entitlement: "included",
      modelPolicy: "included",
      defaultTab: "document",
      defaultWorkflow: "spec:spec",
      skillBindings: { "spec:spec": "spec:spec" },
      tabs: [{ id: "document", label: "Document" }],
      slashCommands: [{
        prefixes: ["/spec", "$spec:spec"],
        tab: "document",
        workflow: "spec:spec",
      }],
    },
    {
      kind: "scout",
      version: "1.0.0",
      label: "Scout",
      headline: "Map how your systems really work",
      value: "Evidence-backed system maps and change impact.",
      engine: "skill",
      entitlement: "subscription",
      modelPolicy: "specialist",
      defaultTab: "map",
      defaultWorkflow: "scout:scout",
      skillBindings: { "scout:scout": "scout:scout" },
      tabs: [
        { id: "map", label: "System map" },
        { id: "changes", label: "Changes" },
        { id: "simulations", label: "Simulations" },
        { id: "evidence", label: "Evidence" },
        { id: "runs", label: "Runs" },
      ],
      slashCommands: [{
        prefixes: ["/scout", "$scout:scout"],
        tab: "map",
        workflow: "scout:scout",
      }],
    },
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
    {
      kind: "rsi",
      version: "1.0.0",
      label: "RSI",
      headline: "Turn requests into verified engineering actions",
      value: "An RSI controller grounds the request, Clark Engineer acts, and typed receipts preserve authority and evidence.",
      engine: "research_runtime",
      runtime: { modelRoute: "clark_free" },
      entitlement: "subscription",
      modelPolicy: "specialist",
      defaultTab: "evaluations",
      defaultWorkflow: "rsi:research",
      skillBindings: {},
      tabs: [
        { id: "worlds", label: "Worlds" },
        { id: "evaluations", label: "Evaluations" },
        { id: "runs", label: "Runs" },
        { id: "frontier", label: "Frontier" },
        { id: "evidence", label: "Evidence" },
      ],
      slashCommands: [
        { prefixes: ["/rsi", "/eval-research"], tab: "evaluations", workflow: "rsi:research" },
        { prefixes: ["/create-evals"], tab: "evaluations", workflow: "rsi:create-evals" },
        { prefixes: ["/build-world"], tab: "worlds", workflow: "rsi:build-world" },
        { prefixes: ["/stress-test", "/simulate", "/simulator"], tab: "frontier", workflow: "rsi:stress-test" },
        {
          prefixes: ["/regression-sim"],
          tab: "frontier",
          workflow: "rsi:regression",
          promptPrefix: "Build a deterministic regression evaluation world. ",
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
    providerExtra: ({ specialist }) => specialist
      && (specialist.kind === "scout" || specialist.kind === "security")
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
  specialistWorkspace: {
    isConversationBound: (kind) => kind === "spec" || kind === "scout",
    prepareDocument: async (kind, conversationId, initialDocument) => {
      if (kind === "scout") return null;
      if (kind !== "spec") throw new Error("Unsupported specialist workspace");
      const filename = initialDocument?.filename
        ?? filenames.get(conversationId)
        ?? "untitled-feature_SPEC.md";
      filenames.set(conversationId, filename);
      const receipt: PreparationReceipt = {
        conversationId,
        filename,
        initialDocument: initialDocument ?? null,
      };
      const current = JSON.parse(
        localStorage.getItem(SPECIALIST_MATRIX_PRODUCT_PROBE) ?? "[]",
      ) as PreparationReceipt[];
      localStorage.setItem(
        SPECIALIST_MATRIX_PRODUCT_PROBE,
        JSON.stringify([...current, receipt]),
      );
      return { filename };
    },
  },
});
