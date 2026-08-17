import { describe, expect, it } from "vitest";

import type { TimelineItem } from "../core-bridge/types";
import { scopedSpecPrompt } from "./specDocuments";
import { specSelectionConversations, specSelectionKey } from "./specSelectionThreads";

function message(run: string, role: "user" | "agent", text: string): TimelineItem {
  return { item: "message", run, role, blocks: [{ type: "text", text }] };
}

describe("selection-scoped Spec conversations", () => {
  it("keeps section turns isolated while rebuilding them from the transcript", () => {
    const conversations = specSelectionConversations([
      message("purpose-1", "user", scopedSpecPrompt("Reduce ambiguity.", "Make this measurable.", "Purpose")),
      message("purpose-1", "agent", "I added a time-to-first-contribution measure."),
      message("support-1", "user", scopedSpecPrompt("Make help easy to find.", "Name the owner.", "Support & feedback")),
      message("support-1", "agent", "I named the onboarding buddy as the owner."),
    ]);

    expect(conversations[specSelectionKey("Purpose")].turns).toEqual([{
      question: "Make this measurable.",
      reply: "I added a time-to-first-contribution measure.",
      runId: "purpose-1",
    }]);
    expect(conversations[specSelectionKey("Support & feedback")].turns[0].reply)
      .toBe("I named the onboarding buddy as the owner.");
  });

  it("ignores ordinary whole-spec conversation turns", () => {
    expect(specSelectionConversations([
      message("whole", "user", "Rewrite the whole specification."),
      message("whole", "agent", "Done."),
    ])).toEqual({});
  });
});
