import {
  installProductModule,
  neutralProduct,
  type ProductInitialSpecialistDocument,
} from "../../app/src/product/productModule";

export const SPEC_PRODUCT_PROBE_STORAGE_KEY =
  "agent-desktop:spec-subchat-product-probe";
export const SPEC_PRODUCT_CONTROL_STORAGE_KEY =
  "agent-desktop:spec-subchat-product-control";

interface PreparationReceipt {
  conversationId: string;
  filename: string;
  initialDocument: ProductInitialSpecialistDocument | null;
}

const filenames = new Map<string, string>();

function recordPreparation(receipt: PreparationReceipt): void {
  const current = JSON.parse(
    localStorage.getItem(SPEC_PRODUCT_PROBE_STORAGE_KEY) ?? "[]",
  ) as PreparationReceipt[];
  localStorage.setItem(
    SPEC_PRODUCT_PROBE_STORAGE_KEY,
    JSON.stringify([...current, receipt]),
  );
}

installProductModule({
  ...neutralProduct,
  branding: {
    id: "spec_e2e",
    name: "Clark Code",
    shortName: "Clark",
  },
  authRequired: true,
  specialistCatalog: {
    schemaVersion: 1,
    catalogVersion: "1.0.0",
    catalogSha256: "0".repeat(64),
    trust: {
      source: "signed_app_bundle",
      requiresSignedReleaseBinary: true,
    },
    manifests: [{
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
    }],
  },
  specialistWorkspace: {
    isConversationBound: (kind) => kind === "spec",
    prepareDocument: async (kind, conversationId, initialDocument) => {
      if (kind !== "spec") return null;
      const control = JSON.parse(
        localStorage.getItem(SPEC_PRODUCT_CONTROL_STORAGE_KEY) ?? "{}",
      ) as { prepareDocument?: "pass" | "fail" };
      if (control.prepareDocument === "fail") {
        throw new Error("fixture document preparation failed");
      }
      const filename = initialDocument?.filename
        ?? filenames.get(conversationId)
        ?? "untitled-feature_SPEC.md";
      filenames.set(conversationId, filename);
      recordPreparation({
        conversationId,
        filename,
        initialDocument: initialDocument ?? null,
      });
      return { filename };
    },
  },
});
