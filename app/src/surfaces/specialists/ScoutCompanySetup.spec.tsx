import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { CompanyScoutSetupControl, CompanyScoutSetupNotice } from "./ScoutCompanySetup";
import { ScoutScopeDialog } from "./ScoutScopeDialog";

const base = {
  organizationId: "org-1",
  companyScoutReady: false,
  serverReady: true,
  bound: false,
  settingUp: false,
  onSetup: vi.fn(),
};

describe("Company Scout setup", () => {
  it("offers setup to company administrators", () => {
    const markup = renderToStaticMarkup(createElement(CompanyScoutSetupControl, {
      ...base,
      organizations: [{ id: "org-1", name: "Example", role: "owner", status: "active" }],
    }));
    expect(markup).toContain("Set up Company Scout");
  });

  it("does not offer an action the server will reject to ordinary members", () => {
    const markup = renderToStaticMarkup(createElement(CompanyScoutSetupControl, {
      ...base,
      organizations: [{ id: "org-1", name: "Example", role: "member", status: "active" }],
    }));
    expect(markup).toContain("Ask a company admin");
    expect(markup).not.toContain("<button");
  });

  it("does not expose a second control after Company Scout exists", () => {
    const markup = renderToStaticMarkup(createElement(CompanyScoutSetupControl, {
      ...base,
      companyScoutReady: true,
      organizations: [{ id: "org-1", name: "Example", role: "owner", status: "active" }],
    }));
    expect(markup).toBe("");
  });

  it("renders setup failures in the visible company flow", () => {
    const markup = renderToStaticMarkup(createElement(CompanyScoutSetupNotice, {
      notice: { tone: "error", message: "administrator access is required" },
      onDismiss: vi.fn(),
    }));
    expect(markup).toContain('role="alert"');
    expect(markup).toContain("administrator access is required");
  });

  it("uses one company selector and no map selector", () => {
    const markup = renderToStaticMarkup(createElement(ScoutScopeDialog, {
      organizations: [{ id: "org-1", name: "Example", role: "owner", status: "active" }],
      companyScoutReady: true,
      organizationId: "org-1",
      loading: false,
      settingUpCompanyScout: false,
      onSelectOrganization: vi.fn(),
      onCreateOrganization: vi.fn(),
      onSetupCompanyScout: vi.fn(),
      onClose: vi.fn(),
    }));
    expect(markup.match(/<select/g)).toHaveLength(1);
    expect(markup).toContain("Choose company");
    expect(markup).toContain("one shared map");
    expect(markup).not.toContain("workspace");
  });
});
