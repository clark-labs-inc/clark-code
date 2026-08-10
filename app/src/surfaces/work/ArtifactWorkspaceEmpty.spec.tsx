import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { ArtifactWorkspaceEmpty } from "./ArtifactWorkspaceEmpty";

describe("ArtifactWorkspaceEmpty", () => {
  it("explains the empty state and keeps the panel dismissible", () => {
    const html = renderToStaticMarkup(<ArtifactWorkspaceEmpty onClose={vi.fn()} />);

    expect(html).toContain('aria-label="Artifact workspace"');
    expect(html).toContain('aria-label="Close artifact workspace"');
    expect(html).toContain("No artifacts yet");
    expect(html).toContain("Files and other outputs created in this task will appear here.");
  });
});
