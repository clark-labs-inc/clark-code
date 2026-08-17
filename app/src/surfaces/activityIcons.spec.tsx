import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { ToolCall } from "../core-bridge/types";
import { Message } from "./Message";
import { ResearchOutline } from "./work/ResearchWork";
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

  it("renders research as a compact work receipt", () => {
    const markup = renderToStaticMarkup(
      <WorkLine
        call={toolCall({
          kind: "research",
          title: "brokered_research: WKWebView selection",
          raw_input: { query: "WKWebView selection" },
        })}
        active={false}
      />,
    );

    expect(markup).toContain("WKWebView selection");
    expect(markup).toContain('data-clark-work-receipt="true"');
    expect(markup).toContain("lucide-earth");
    expect(markup).not.toContain("Research agent");
    expect(markup).not.toContain("lucide-telescope");
  });

  it("shows a quiet starting state before the first public progress event", () => {
    const markup = renderToStaticMarkup(
      <WorkLine
        call={toolCall({
          kind: "research",
          status: "in_progress",
          title: "brokered_research: WKWebView selection",
          raw_input: { query: "WKWebView selection" },
        })}
        active
      />,
    );

    expect(markup).toContain("Live");
    expect(markup).toContain("WKWebView selection");
    expect(markup).not.toContain("Web search");
    expect(markup).not.toContain("Source reading");
    expect(markup).not.toContain("Cited synthesis");
  });

  it("renders the public run hierarchy and expands only the current phase", () => {
    const progress = {
      revision: 6,
      status: "in_progress" as const,
      latest_activity: "Reading API and architecture pages",
      phases: [
        { id: "plan", title: "Plan research", status: "completed" as const, steps: [] },
        {
          id: "verify",
          title: "Search and verify sources",
          status: "in_progress" as const,
          steps: [
            { id: "search", title: "Search official Vorflux sources", status: "completed" as const },
            {
              id: "read",
              title: "Read example.test",
              status: "in_progress" as const,
              summary: "Reading API and architecture pages",
            },
            { id: "cross-check", title: "Cross-check product claims", status: "pending" as const },
          ],
        },
        {
          id: "synthesize",
          title: "Synthesize findings",
          status: "pending" as const,
          steps: [{ id: "hidden", title: "Hidden pending detail", status: "pending" as const }],
        },
      ],
      agents: [
        {
          id: "docs",
          label: "Vorflux documentation",
          status: "in_progress" as const,
          activity: "Reading API and architecture pages",
        },
        {
          id: "product",
          label: "the agent product site",
          status: "completed" as const,
          summary: "Verified primary claims",
        },
      ],
    };
    const markup = renderToStaticMarkup(<ResearchOutline progress={progress} />);

    expect(markup).toContain("Reading API and architecture pages");
    expect(markup).toContain("Plan research");
    expect(markup).toContain("Search and verify sources");
    expect(markup).toContain("Search official Vorflux sources");
    expect(markup).toContain("Cross-check product claims");
    expect(markup).toContain("Synthesize findings");
    expect(markup).not.toContain("Hidden pending detail");
    expect(markup).toContain("Parallel research · 2 agents");
    expect(markup).toContain("Vorflux documentation");
    expect(markup).toContain("the agent product site");
    expect(markup).toContain("Verified primary claims");
  });

  it("preserves a completed outline after the final textual result", () => {
    const markup = renderToStaticMarkup(
      <ResearchOutline
        progress={{
          revision: 9,
          status: "completed",
          latest_activity: "Research complete",
          phases: [
            { id: "plan", title: "Plan research", status: "completed", steps: [] },
            { id: "verify", title: "Verify sources", status: "completed", steps: [] },
          ],
          agents: [],
        }}
      />,
    );

    expect(markup).toContain("Plan research");
    expect(markup).toContain("Verify sources");
  });

  it("derives the completed source count from returned findings", () => {
    const markup = renderToStaticMarkup(
      <WorkLine
        call={toolCall({
          kind: "research",
          title: "brokered_research: WKWebView selection",
          raw_input: { query: "WKWebView selection" },
          content: [{ type: "text", text: "See https://webkit.org/ and https://github.com/WebKit/WebKit." }],
        })}
        active={false}
      />,
    );

    expect(markup).toContain("2 sources");
    expect(markup).toContain('data-clark-work-receipt="true"');
  });

  it("shows explicit failed and cancelled terminal states", () => {
    const failed = renderToStaticMarkup(
      <WorkLine
        call={toolCall({
          kind: "research",
          status: "failed",
          title: "brokered_research: Verify sources",
          progress: {
            revision: 3,
            status: "failed",
            latest_activity: "Research failed safely",
            phases: [],
            agents: [],
          },
        })}
        active={false}
      />,
    );
    const cancelled = renderToStaticMarkup(
      <WorkLine
        call={toolCall({
          kind: "research",
          status: "cancelled",
          title: "brokered_research: Verify sources",
        })}
        active={false}
      />,
    );

    expect(failed).toContain("Research failed safely");
    expect(failed).toContain("Failed");
    expect(cancelled).toContain("Research cancelled");
    expect(cancelled).toContain("Cancelled");
  });

  it("omits reasoning-only messages from the activity UI", () => {
    const markup = renderToStaticMarkup(
      <Message
        role="agent"
        blocks={[{ type: "thinking", text: "Inspect the activity feed." }]}
        timelineIndex={0}
      />,
    );

    expect(markup).toBe("");
  });
});
