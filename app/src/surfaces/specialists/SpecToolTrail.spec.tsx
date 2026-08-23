import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { ToolCall, ToolKind, ToolStatus } from "../../core-bridge/types";
import { SpecToolList, SpecToolTrail } from "./SpecToolTrail";

function call(id: string, kind: ToolKind, status: ToolStatus, over: Partial<ToolCall> = {}): ToolCall {
  return { id, title: `${id} title`, kind, status, locations: [], content: [], ...over };
}

describe("SpecToolTrail", () => {
  it("shows one kind glyph per call of the turn", () => {
    const markup = renderToStaticMarkup(
      <SpecToolTrail
        calls={[
          call("read", "read", "completed"),
          call("edit", "edit", "in_progress"),
          call("research", "research", "pending"),
        ]}
      />,
    );

    expect(markup).toContain("lucide-book-open");
    expect(markup).toContain("lucide-pencil-line");
    // Globe2 renders as the canonical `earth`, matching the research row in chat.
    expect(markup).toContain("lucide-earth");
  });

  it("changes shape for a failure, not just colour", () => {
    // html.colorblind swaps success/danger to blue/orange, so hue alone cannot
    // carry this.
    const markup = renderToStaticMarkup(
      <SpecToolTrail calls={[call("search", "search", "failed")]} />,
    );

    expect(markup).toContain("lucide-triangle-alert");
    expect(markup).not.toContain("lucide-search");
  });

  it("names each glyph with its kind and status for assistive tech", () => {
    const markup = renderToStaticMarkup(
      <SpecToolTrail calls={[call("edit", "edit", "in_progress", { title: "apply_patch: Writing the draft" })]} />,
    );

    expect(markup).toContain('aria-label="Edit: Writing the draft — Running"');
  });

  it("elides the head past the window and says how many are hidden", () => {
    const calls = Array.from({ length: 10 }, (_, i) => call(`c${i}`, "read", "completed"));

    const markup = renderToStaticMarkup(<SpecToolTrail calls={calls} />);

    expect(markup).toContain("+3");
  });

  it("renders nothing before the first tool call", () => {
    expect(renderToStaticMarkup(<SpecToolTrail calls={[]} />)).toBe("");
  });
});

describe("SpecToolList", () => {
  it("lists each call with its target and status", () => {
    const markup = renderToStaticMarkup(
      <SpecToolList
        calls={[
          call("read", "read", "completed", { locations: [{ path: "new_SPEC.md" }] }),
          call("run", "execute", "in_progress", { title: "bash: cargo check" }),
        ]}
      />,
    );

    expect(markup).toContain("new_SPEC.md");
    expect(markup).toContain("cargo check");
    expect(markup).toContain("Complete");
    expect(markup).toContain("Running");
    expect(markup).toContain("Command");
  });

  it("nests the delegated outline only when the call reports one", () => {
    const withProgress = renderToStaticMarkup(
      <SpecToolList
        calls={[call("research", "research", "in_progress", {
          progress: {
            revision: 2,
            status: "in_progress",
            latest_activity: "Reading the API reference",
            phases: [{ id: "verify", title: "Verify sources", status: "in_progress", steps: [] }],
            agents: [],
          },
        })]}
      />,
    );

    expect(withProgress).toContain("Verify sources");
  });

  it("does not strand a spinner on a finished call that reports no progress", () => {
    // ResearchOutline shows "Starting research agent" whenever progress is
    // absent, whatever the call's status — so the caller must gate on it.
    const markup = renderToStaticMarkup(
      <SpecToolList calls={[call("research", "research", "completed")]} />,
    );

    expect(markup).not.toContain("Starting research agent");
    expect(markup).not.toContain("lucide-loader-circle");
  });
});
