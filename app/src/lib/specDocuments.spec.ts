import { describe, expect, it } from "vitest";
import {
  initialSpecMarkdown,
  latestSpecArtifact,
  parseScopedSpecPrompt,
  scopedSpecPrompt,
  specCodeContextPrompt,
  specDocumentTitle,
  specFilename,
  specPathWithinRepository,
  specRelativePath,
  specRepositoryLabel,
} from "./specDocuments";

describe("spec document conventions", () => {
  it("starts without template boilerplate or a prompt-derived title", () => {
    expect(initialSpecMarkdown("A raw submitted prompt")).toBe("");
  });

  it("creates semantic Markdown and PDF filenames", () => {
    expect(specFilename("Customer Segmentation Spec", "md"))
      .toBe("customer-segmentation_SPEC.md");
    expect(specFilename("Customer Segmentation Spec", "pdf"))
      .toBe("customer-segmentation_SPEC.pdf");
  });

  it("projects the completed document H1 into the saved Spec title", () => {
    expect(specDocumentTitle("# Package-Proven RSI Recovery — Product Specification\n\n## Goals"))
      .toBe("Package-Proven RSI Recovery");
    expect(specDocumentTitle("# Offline Draft Recovery - Engineering Spec\n"))
      .toBe("Offline Draft Recovery");
    expect(specDocumentTitle("# Feature Specification: Shareable links\n"))
      .toBe("Shareable links");
    expect(specDocumentTitle("# Untitled feature\n")).toBeNull();
  });

  it("prefers the latest semantic spec artifact", () => {
    expect(latestSpecArtifact([
      { id: "notes", title: "notes.md", kind: "file", uri: "/tmp/notes.md" },
      { id: "spec", title: "customer-segmentation_SPEC.md", kind: "file", uri: "/tmp/customer-segmentation_SPEC.md" },
    ])?.id).toBe("spec");
  });

  it("binds a scoped chat to exact selected content", () => {
    const prompt = scopedSpecPrompt("Do not drag right.", "Should it wrap?", "Drag behavior");
    expect(prompt).toContain("<selected_spec_section>\nDrag behavior\n</selected_spec_section>");
    expect(prompt).toContain("<selected_spec_content>\nDo not drag right.\n</selected_spec_content>");
    expect(prompt).toContain("<scoped_comment>\nShould it wrap?\n</scoped_comment>");
    expect(prompt).toContain("Preserve unrelated sections");
    expect(parseScopedSpecPrompt(prompt)).toEqual({
      selection: "Do not drag right.",
      section: "Drag behavior",
      comment: "Should it wrap?",
    });
  });

  it("grounds referenced code inside the selected repository", () => {
    expect(specPathWithinRepository("/repo/clark", "/repo/clark/app/src")).toBe(true);
    expect(specPathWithinRepository("/repo/clark", "/repo/clark-other/app")).toBe(false);
    expect(specRelativePath("/repo/clark", "/repo/clark/app/src")).toBe("app/src");
    expect(specRepositoryLabel("/repo/clark/")).toBe("clark");
  });

  it("asks the Spec workflow to read referenced code without implementing it", () => {
    const prompt = specCodeContextPrompt("Explain the current empty state.", "/repo/clark", [
      { kind: "folder", path: "app/src/surfaces" },
      { kind: "file", path: "app/src/App.tsx" },
    ]);
    expect(prompt.startsWith("Explain the current empty state.")).toBe(true);
    expect(prompt).toContain('"repository_root": "/repo/clark"');
    expect(prompt).toContain('"path": "app/src/surfaces"');
    expect(prompt).toContain("Inspect the referenced files or folders before changing");
    expect(prompt).toContain("not permission to implement anything");
  });
});
