import { describe, expect, it } from "vitest";

import { specInteractionActions } from "./specInteractions";

describe("specInteractionActions", () => {
  it("turns open questions into decision-oriented actions", () => {
    const actions = specInteractionActions(
      "OQ1 What is the primary scoring signal for produced code? (a) hidden tests (b) convergence",
    );

    expect(actions.map((action) => action.label)).toEqual([
      "Recommend",
      "Record decision",
      "Clarify",
    ]);
    expect(actions.find((action) => action.id === "decide")?.prompt).toContain("Record this decision");
  });

  it("offers review and revision actions for ordinary spec prose", () => {
    const actions = specInteractionActions("The document remains available while Clark is working.");

    expect(actions.map((action) => action.label)).toEqual([
      "Ask about this",
      "Suggest edit",
    ]);
  });
});
