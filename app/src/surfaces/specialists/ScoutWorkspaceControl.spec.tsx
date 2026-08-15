import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { ScoutWorkspaceControl, ScoutWorkspaceNotice } from "./ScoutWorkspaceControl";

const base = {
  organizationId: "org-1",
  workspaces: [],
  serverReady: true,
  bound: false,
  creating: false,
  onSelect: vi.fn(),
  onCreate: vi.fn(),
};

describe("Scout workspace controls", () => {
  it("offers creation to organization administrators", () => {
    const markup = renderToStaticMarkup(createElement(ScoutWorkspaceControl, {
      ...base,
      organizations: [{ id: "org-1", name: "Example", role: "owner", status: "active" }],
    }));
    expect(markup).toContain("Create workspace");
  });

  it("does not offer an action the server will reject to ordinary members", () => {
    const markup = renderToStaticMarkup(createElement(ScoutWorkspaceControl, {
      ...base,
      organizations: [{ id: "org-1", name: "Example", role: "member", status: "active" }],
    }));
    expect(markup).toContain("Ask an organization admin");
    expect(markup).not.toContain("<button");
  });

  it("renders create failures in the visible workspace, not only hidden insights", () => {
    const markup = renderToStaticMarkup(createElement(ScoutWorkspaceNotice, {
      notice: { tone: "error", message: "administrator access is required" },
      onDismiss: vi.fn(),
    }));
    expect(markup).toContain('role="alert"');
    expect(markup).toContain("administrator access is required");
  });
});
