import { describe, expect, it } from "vitest";
import workspaceSource from "./AuthenticatedWorkspace.tsx?raw";

describe("AuthenticatedWorkspace artifact navigation", () => {
  it("keeps the artifact surface inside the current conversation and specialist workspace", () => {
    expect(workspaceSource).not.toContain("if (!latest) return;");
    expect(workspaceSource).toContain("setArtifactPanelOpen(true);");
    expect(workspaceSource).toContain("<ArtifactWorkspaceEmpty");
    expect(workspaceSource).toContain("<SpecialistWorkspace dark={dark}");
    expect(workspaceSource).toContain("onOpenArtifact={openArtifact}");
    expect(workspaceSource).not.toContain("useSpecialistStore.getState().close();");
    expect(workspaceSource).toContain("if (specialist) newSpecialistConversation(specialist);");
  });
});
