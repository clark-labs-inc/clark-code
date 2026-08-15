import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import conversationSource from "../Conversation.tsx?raw";
import { ArtifactCard } from "./ArtifactCard";

describe("ArtifactCard", () => {
  it("keeps the compact artifact edge treatment uniform", () => {
    const markup = renderToStaticMarkup(
      <ArtifactCard
        artifact={{
          id: "artifact-1",
          title: "Artifact UX recommendations.md",
          kind: "file",
          mime_type: "text/markdown",
          uri: "data:text/markdown,%23%20Artifact%20UX",
        }}
      />,
    );

    expect(markup).toContain("border-y border-border-subtle");
    expect(markup).not.toContain("border-accent");
    expect(markup).not.toContain("inset_3px");
  });

  it("highlights the artifact header without drawing a fragmented outer focus ring", () => {
    const markup = renderToStaticMarkup(
      <ArtifactCard
        artifact={{
          id: "artifact-image-1",
          title: "artifact-preview.svg",
          kind: "image",
          mime_type: "image/svg+xml",
          uri: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg'/%3E",
        }}
      />,
    );

    expect(conversationSource).toContain('className="group/artifact relative outline-none"');
    expect(conversationSource).not.toContain("focus-visible:ring-2 focus-visible:ring-accent");
    expect(markup).toContain("group-focus-visible/artifact:text-ink");
  });

  it("renders visual artifacts as a Canvas Drop with focused workspace actions", () => {
    const markup = renderToStaticMarkup(
      <ArtifactCard
        artifact={{
          id: "artifact-image-2",
          title: "artifact-preview.svg",
          kind: "image",
          mime_type: "image/svg+xml",
          uri: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg'/%3E",
        }}
        onOpen={() => undefined}
      />,
    );

    expect(markup).toContain('data-qa="artifact-canvas-drop"');
    expect(markup).toContain("Artifact Preview");
    expect(markup).toContain("artifact-preview.svg");
    expect(markup).toContain("SVG");
    expect(markup).toContain("Ready to review");
    expect(markup).toContain("from");
    expect(markup).toContain("Open workspace");
    expect(markup).toContain("Save a copy");
    expect(markup).not.toContain("border-y border-border-subtle");
  });
});
