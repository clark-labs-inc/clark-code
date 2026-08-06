import { describe, expect, it } from "vitest";
import { changesBaseForRuns } from "./ChangesPanel";

describe("Changes baseline", () => {
  it("resets a checkpoint that belongs to the previous conversation", () => {
    expect(changesBaseForRuns("old-session-base", ["new-session-base", "new-turn"])).toBe(
      "new-session-base",
    );
  });

  it("preserves a still-valid user-selected checkpoint", () => {
    expect(changesBaseForRuns("new-turn", ["new-session-base", "new-turn"])).toBe("new-turn");
  });
});
