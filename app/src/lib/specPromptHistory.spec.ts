import { describe, expect, it, vi } from "vitest";
import type { TimelineItem } from "../core-bridge/types";
import {
  loadSpecPromptHistory,
  recentSpecPrompts,
  recordSpecPrompt,
  visibleSpecPrompt,
} from "./specPromptHistory";

describe("Spec prompt history", () => {
  it("keeps user copy while hiding the repository context envelope", () => {
    expect(visibleSpecPrompt(`Explain the empty state.

Continue the feature-specification workflow for the current SPEC.md.

<spec_code_context>{}</spec_code_context>`)).toBe("Explain the empty state.");
  });

  it("stores a small account-and-conversation-scoped prompt history", () => {
    vi.spyOn(Date, "now").mockReturnValue(42);
    recordSpecPrompt("spec-history-test-owner-one", "spec-history-test-one", "Keep my prompt visible");

    expect(loadSpecPromptHistory("spec-history-test-owner-one", "spec-history-test-one")).toEqual([{
      text: "Keep my prompt visible",
      submittedAt: 42,
    }]);
    expect(loadSpecPromptHistory("spec-history-test-owner-two", "spec-history-test-one")).toEqual([]);
  });

  it("merges restored transcript prompts with locally retained submissions", () => {
    const timeline: TimelineItem[] = [{
      item: "message",
      run: "run-1",
      role: "user",
      blocks: [{ type: "text", text: "Earlier prompt" }],
    }];

    expect(recentSpecPrompts(
      [{ text: "Prompt retained through refresh", submittedAt: 10 }],
      timeline,
    ).map((item) => item.text)).toEqual([
      "Earlier prompt",
      "Prompt retained through refresh",
    ]);
  });
});
