import { describe, expect, it } from "vitest";
import { composerBrandingCopy } from "./composerBranding";

describe("composer product branding", () => {
  it("names Clark Code instead of the generic desktop agent", () => {
    expect(composerBrandingCopy("Clark Code")).toEqual({
      ariaLabel: "Message Clark Code",
      initialPlaceholder: "Describe what you want Clark Code to do…",
      projectPlaceholder: "Ask Clark Code anything about this project…",
      goalHelp: "Describe what Clark Code should keep working toward after /goal.",
      goalStatus: "Clark Code keeps going until it is done",
      queuedTitle: "Queue message (sends when Clark Code finishes)",
    });
  });
});
