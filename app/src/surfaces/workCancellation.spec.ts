import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { ToolCall } from "../core-bridge/types";
import { WorkLine } from "./work/WorkLine";

function cancelledCall(overrides: Partial<ToolCall> = {}): ToolCall {
  return {
    id: "cancelled-call",
    title: "Read https://example.com",
    kind: "fetch",
    status: "cancelled",
    locations: [],
    content: [],
    ...overrides,
  };
}

describe("cancelled tool presentation", () => {
  it("keeps cancelled work visibly distinct from completed work", () => {
    const markup = renderToStaticMarkup(
      createElement(WorkLine, { call: cancelledCall(), active: false }),
    );

    expect(markup).toContain('aria-label="cancelled"');
  });

  it("does not present cancelled research as complete", () => {
    const markup = renderToStaticMarkup(
      createElement(WorkLine, {
        call: cancelledCall({
          kind: "research",
          title: "brokered_research: interrupted lookup",
        }),
        active: false,
      }),
    );

    expect(markup).toContain("Cancelled");
    expect(markup).not.toContain(">Complete<");
  });
});
