import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { ToolCall } from "../core-bridge/types";
import { Message } from "./Message";
import { WorkLine } from "./work/WorkLine";

function toolCall(overrides: Partial<ToolCall> = {}): ToolCall {
  return {
    id: "call-1",
    title: "grep: app/src",
    kind: "search",
    status: "completed",
    locations: [],
    content: [],
    ...overrides,
  };
}

describe("activity icon restraint", () => {
  it("renders ordinary work rows without a leading kind icon", () => {
    const markup = renderToStaticMarkup(<WorkLine call={toolCall()} active={false} />);

    expect(markup).toContain("grep: app/src");
    expect(markup).not.toContain("lucide-search");
    expect(markup).not.toContain("lucide-square-terminal");
  });

  it("renders the Clark Cloud identity for research", () => {
    const markup = renderToStaticMarkup(
      <WorkLine
        call={toolCall({
          kind: "research",
          title: "clark_research: WKWebView selection",
          raw_input: { query: "WKWebView selection" },
        })}
        active={false}
      />,
    );

    expect(markup).toContain("Clark Cloud Agent");
    expect(markup).toContain("Running securely on clarkchat.com");
    expect(markup).toContain('aria-label="Clark"');
    expect(markup).not.toContain("lucide-telescope");
  });

  it("shows the compact cloud process without invented telemetry", () => {
    const markup = renderToStaticMarkup(
      <WorkLine
        call={toolCall({
          kind: "research",
          status: "in_progress",
          title: "clark_research: WKWebView selection",
          raw_input: { query: "WKWebView selection" },
        })}
        active
      />,
    );

    expect(markup).toContain("Live");
    expect(markup).toContain("Plan");
    expect(markup).toContain("Search");
    expect(markup).toContain("Read");
    expect(markup).toContain("Synthesize");
    expect(markup).toContain("Web search");
    expect(markup).not.toContain("threads");
    expect(markup).not.toContain("lines reviewed");
  });

  it("derives the completed source count from returned findings", () => {
    const markup = renderToStaticMarkup(
      <WorkLine
        call={toolCall({
          kind: "research",
          title: "clark_research: WKWebView selection",
          raw_input: { query: "WKWebView selection" },
          content: [{ type: "text", text: "See https://webkit.org/ and https://github.com/WebKit/WebKit." }],
        })}
        active={false}
      />,
    );

    expect(markup).toContain("2 sources");
    expect(markup).toContain("View research brief");
  });

  it("renders thinking as text and disclosure only", () => {
    const markup = renderToStaticMarkup(
      <Message
        role="agent"
        blocks={[{ type: "thinking", text: "Inspect the activity feed." }]}
        timelineIndex={0}
      />,
    );

    expect(markup).toContain("Thinking");
    expect(markup).toContain('aria-expanded="false"');
    expect(markup).not.toContain("lucide-brain");
  });
});
