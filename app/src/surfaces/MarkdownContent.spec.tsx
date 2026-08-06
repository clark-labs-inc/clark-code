import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { MarkdownContent } from "./MarkdownContent";

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
});
