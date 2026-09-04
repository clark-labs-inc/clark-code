import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { runSpecialistGateAction } from "./SpecialistAccessGate";

describe("specialist access gate actions", () => {
  it("routes a product action to billing instead of retrying access", () => {
    const handlers = {
      signIn: vi.fn(),
      retry: vi.fn(),
      productAction: vi.fn(),
      setupWorkspace: vi.fn(),
    };

    runSpecialistGateAction("product_action", handlers);

    expect(handlers.productAction).toHaveBeenCalledOnce();
    expect(handlers.retry).not.toHaveBeenCalled();
    expect(handlers.signIn).not.toHaveBeenCalled();
    expect(handlers.setupWorkspace).not.toHaveBeenCalled();
  });

  it("routes workspace setup to the product's explicit Scout setup flow", () => {
    const handlers = {
      signIn: vi.fn(),
      retry: vi.fn(),
      productAction: vi.fn(),
      setupWorkspace: vi.fn(),
    };

    runSpecialistGateAction("setup_workspace", handlers);

    expect(handlers.setupWorkspace).toHaveBeenCalledOnce();
    expect(handlers.productAction).not.toHaveBeenCalled();
  });

  it("renders failed verification as a retryable Security state", async () => {
    vi.resetModules();
    const { installProductModule, neutralProduct } = await import("../../product/productModule");
    installProductModule({
      ...neutralProduct,
      branding: { id: "security_gate_test", name: "Clark", shortName: "Clark" },
      authRequired: true,
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
          value: "Verified findings, safe PoCs, and remediation.",
          engine: "skill",
          entitlement: "subscription",
          modelPolicy: "specialist",
          tabs: [{ id: "posture", label: "Posture" }],
          defaultTab: "posture",
          defaultWorkflow: "security:security-scan",
          skillBindings: { "security:security-scan": "security:security-scan" },
          slashCommands: [{
            prefixes: ["/security"],
            tab: "posture",
            workflow: "security:security-scan",
          }],
        }],
      },
    });
    const { SpecialistAccessGate } = await import("./SpecialistAccessGate");
    const markup = renderToStaticMarkup(createElement(SpecialistAccessGate, {
      kind: "security",
      state: "offline",
      onRetry: vi.fn(),
      onProductAction: vi.fn(),
      onWorkspaceSetup: vi.fn(),
    }));

    expect(markup).toContain("Security could not verify access");
    expect(markup).toContain("Try again");
    expect(markup).not.toContain("Checking Security access");
  });

  it("routes the failed-verification action only to retry", () => {
    const handlers = {
      signIn: vi.fn(),
      retry: vi.fn(),
      productAction: vi.fn(),
      setupWorkspace: vi.fn(),
    };

    runSpecialistGateAction("retry", handlers);

    expect(handlers.retry).toHaveBeenCalledOnce();
    expect(handlers.signIn).not.toHaveBeenCalled();
    expect(handlers.productAction).not.toHaveBeenCalled();
    expect(handlers.setupWorkspace).not.toHaveBeenCalled();
  });
});
