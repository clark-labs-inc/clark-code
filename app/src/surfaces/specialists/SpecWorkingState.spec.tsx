import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { SpecWorkingState } from "./SpecWorkingState";

describe("SpecWorkingState", () => {
  it("shows simple draft progress without exposing backend tool steps", () => {
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
    expect(markup).toContain("Writing the first draft");
    expect(markup).toContain("Draft progress");
    expect(markup).toContain('role="progressbar"');
    expect(markup).not.toContain("Searching supported repositories");
    expect(markup).not.toContain("Searching for supported repositories");
    expect(markup).not.toContain("Problem and outcome");
  });

  it("keeps the untouched state empty and inviting", () => {
    const markup = renderToStaticMarkup(
      <SpecWorkingState
        hasSubmittedPrompt={false}
        calls={[]}
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
        activity={{ busy: false, label: "Ready" }}
        documentUnavailable
      />,
    );

    expect(markup).toContain("This spec couldn’t be opened");
    expect(markup).toContain("saved document was not replaced");
    expect(markup).not.toContain("Start with the change");
  });
});
