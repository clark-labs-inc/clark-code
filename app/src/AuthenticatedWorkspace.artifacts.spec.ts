import { describe, expect, it } from "vitest";
import workspaceSource from "./AuthenticatedWorkspace.tsx?raw";

describe("AuthenticatedWorkspace artifact navigation", () => {
  it("opens the artifact surface when the current conversation has no artifacts", () => {
    expect(workspaceSource).not.toContain("if (!latest) return;");
    expect(workspaceSource).toContain("setArtifactPanelOpen(true);");
    expect(workspaceSource).toContain("artifactPanelOpen && !session");
    expect(workspaceSource).toContain("<ArtifactWorkspaceEmpty");
  });
});
