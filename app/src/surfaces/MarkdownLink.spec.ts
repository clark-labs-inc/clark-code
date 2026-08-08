import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it } from "vitest";
import { useSessionStore } from "../store/sessionStore";
import { MarkdownLink } from "./MarkdownLink";

beforeEach(() => {
  useSessionStore.setState({
    activeProjectRoot: "/workspace/project",
    activeRemote: null,
  });
});

describe("MarkdownLink", () => {
  it("marks filesystem destinations as native local-file links", () => {
    const markup = renderToStaticMarkup(
      createElement(
        MarkdownLink,
        { href: "/workspace/project/docs/report.docx" },
        "Generated report",
      ),
    );

    expect(markup).toContain('data-local-file="/workspace/project/docs/report.docx"');
    expect(markup).toContain("Right-click for file actions");
    expect(markup).not.toContain('target="_blank"');
  });

  it("keeps web destinations as external links", () => {
    const markup = renderToStaticMarkup(
      createElement(MarkdownLink, { href: "https://example.com/report" }, "Web report"),
    );

    expect(markup).toContain('href="https://example.com/report"');
    expect(markup).toContain('target="_blank"');
    expect(markup).not.toContain("data-local-file");
  });
});
