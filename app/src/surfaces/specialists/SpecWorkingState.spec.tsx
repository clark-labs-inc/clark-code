import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { SpecWorkingState } from "./SpecWorkingState";

const idle = { label: "Ready", source: "unknown" } as const;

describe("SpecWorkingState", () => {
  // Superseded a "without exposing backend tool steps" contract on purpose: the
  // wait before the first draft is the longest stare in the flow, and an
  // indefinite "Writing the first draft…" through all of it reads as a hang. The
  // hero now names the real activity and shows which tools ran.
  it("shows what it is doing while the first draft is still coming", () => {
    const markup = renderToStaticMarkup(
      <SpecWorkingState
        hasSubmittedPrompt
        calls={[{
          id: "research",
          title: "Searching supported repositories",
          kind: "research",
          status: "in_progress",
          locations: [],
          content: [],
        }]}
        status={{ label: "Searching for supported repositories", source: "tool_title" }}
        activity={{
          busy: true,
          label: "Searching for supported repositories",
          detail: "https://github.com/example/project",
          progress: 0.5,
          steps: { done: 1, total: 2 },
        }}
      />,
    );

    expect(markup).toContain("Building your spec");
    expect(markup).toContain("Searching for supported repositories");
    expect(markup).toContain("Draft progress");
    expect(markup).toContain('role="progressbar"');
    expect(markup).toContain("Tools used in this run");
    // The document's own scaffolding still stays out of the waiting state.
    expect(markup).not.toContain("Problem and outcome");
  });

  it("keeps the untouched state empty and inviting", () => {
    const markup = renderToStaticMarkup(
      <SpecWorkingState
        hasSubmittedPrompt={false}
        calls={[]}
        status={idle}
        activity={{ busy: false, label: "Ready" }}
      />,
    );

    expect(markup).toContain("Start with the change you want to make");
    expect(markup).toContain("not from a pre-filled template");
  });

  it("shows an honest recovery state when a saved document cannot be read", () => {
    const markup = renderToStaticMarkup(
      <SpecWorkingState
        hasSubmittedPrompt
        calls={[]}
        status={idle}
        activity={{ busy: false, label: "Ready" }}
        documentUnavailable
      />,
    );

    expect(markup).toContain("This spec couldn’t be opened");
    expect(markup).toContain("saved document was not replaced");
    expect(markup).not.toContain("Start with the change");
  });
});
