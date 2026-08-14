import { describe, expect, it } from "vitest";

import { specDocumentInteraction } from "../../lib/specDiff";

describe("SpecWorkspace live editing contract", () => {
  it("locks selection while the agent is changing the document", () => {
    expect(specDocumentInteraction(true)).toEqual({
      ariaBusy: true,
      className: "cursor-wait select-none",
      canSelect: false,
    });
  });

  it("restores ordinary selection when the agent settles", () => {
    expect(specDocumentInteraction(false)).toEqual({
      ariaBusy: false,
      className: "cursor-text select-text",
      canSelect: true,
    });
  });
});
