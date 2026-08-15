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

  it("renders tables as a readable semantic ledger in static and streaming modes", () => {
    const markdown = [
      "| Field | Value |",
      "| --- | --- |",
      "| Vendor ID | `0x17E9` (6121) |",
      "| Connection | Running through USB |",
    ].join("\n");
    const staticMarkup = renderToStaticMarkup(
      <MarkdownContent>{markdown}</MarkdownContent>,
    );
    const streamingMarkup = renderToStaticMarkup(
      <MarkdownContent mode="streaming">{markdown}</MarkdownContent>,
    );
    const mathMarkup = renderToStaticMarkup(
      <MarkdownContent math>{markdown}</MarkdownContent>,
    );

    for (const markup of [staticMarkup, streamingMarkup, mathMarkup]) {
      expect(markup).toContain('data-markdown-table="true"');
      expect(markup).toContain('aria-label="Scrollable table"');
      expect(markup).toContain('class="markdown-data-table"');
      expect(markup).toContain("<thead>");
      expect(markup).toContain("<tbody>");
      expect(markup).toContain("<th>Field</th>");
      expect(markup).toContain("<td>Vendor ID</td>");
      expect(markup).toContain("<code>0x17E9</code>");
    }

    const incompleteMarkup = renderToStaticMarkup(
      <MarkdownContent mode="streaming">
        {"| Field | Value |\n| --- | --- |\n| Vendor ID | `0x17"}
      </MarkdownContent>,
    );
    expect(incompleteMarkup).toContain('data-markdown-table="true"');
    expect(incompleteMarkup).toContain("<td>Vendor ID</td>");
  });
});
