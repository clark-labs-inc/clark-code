import { describe, expect, it } from "vitest";
import workspaceSource from "./AuthenticatedWorkspace.tsx?raw";
import navigationSource from "./lib/useArtifactPanelSelection.ts?raw";

describe("AuthenticatedWorkspace artifact navigation", () => {
  it("keeps the artifact surface inside the current conversation and specialist workspace", () => {
    expect(navigationSource).not.toContain("if (!latest) return;");
    expect(navigationSource).toContain("setArtifactPanelOpen(true);");
    expect(navigationSource).toContain("preferredArtifact(artifacts)");
    expect(navigationSource).toContain("setSelectedArtifact(artifact);");
    expect(workspaceSource).toContain("selectedArtifact={selectedArtifact}");
    expect(workspaceSource).toContain("useArtifactPanelSelection({");
    expect(workspaceSource).toContain("<ArtifactWorkspaceEmpty");
    expect(workspaceSource).toContain("<SpecialistWorkspace dark={dark}");
    expect(workspaceSource).toContain("onOpenArtifact={openArtifact}");
    expect(workspaceSource).not.toContain("useSpecialistStore.getState().close();");
    expect(workspaceSource).toContain("if (specialist) newSpecialistConversation(specialist);");
  });
});
