import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { MarkdownContent } from "./MarkdownContent";
import { useSessionStore } from "../store/sessionStore";

describe("MarkdownContent", () => {
  it("uses the same semantic renderer for static and streaming content", () => {
    const staticMarkup = renderToStaticMarkup(
      <MarkdownContent>Static **meaning**.</MarkdownContent>,
    );
    const streamingMarkup = renderToStaticMarkup(
      <MarkdownContent
        mode="streaming"
        animated={{ animation: "fadeIn", duration: 200, sep: "word", stagger: 18 }}
        isAnimating
      >
        Streaming **meaning**.
      </MarkdownContent>,
    );

    expect(staticMarkup).toContain("<strong>meaning</strong>");
    expect(streamingMarkup).toContain("<strong><span");
    expect(streamingMarkup).toContain('data-sd-animate="true"');
  });

  it("keeps math support on the canonical static renderer", () => {
    const markup = renderToStaticMarkup(
      <MarkdownContent math>{"$x^2$"}</MarkdownContent>,
    );

    expect(markup).toContain("katex");
    expect(markup).toContain("x");
  });

  it("repairs unfinished Markdown when a document requests best-effort rendering", () => {
    const markup = renderToStaticMarkup(
      <MarkdownContent repairIncomplete>{"A **partially written document"}</MarkdownContent>,
    );

    expect(markup).toContain("<strong>partially written document</strong>");
  });

  it("turns a project-local Markdown image into an inline actionable preview", () => {
    useSessionStore.setState({ activeProjectRoot: "/workspace/project", activeRemote: null });

    const markup = renderToStaticMarkup(
      <MarkdownContent>{"![Spectrogram](/workspace/project/results/spectro.png)"}</MarkdownContent>,
    );

    expect(markup).toContain('data-local-image="/workspace/project/results/spectro.png"');
    expect(markup).toContain("Spectrogram");
    expect(markup).toContain("Open");
    expect(markup).toContain("Save a Copy");
    expect(markup).toContain("Copy Path");
  });
});
