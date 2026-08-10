import { describe, expect, it } from "vitest";
import { canOfferPreservingWorktree } from "./BranchPicker";

describe("branch preservation affordance", () => {
  it("does not promise a managed worktree for a remote checkout", () => {
    expect(canOfferPreservingWorktree(true, { id: "remote-worker" })).toBe(false);
  });

  it("offers a managed worktree for a dirty local checkout", () => {
    expect(canOfferPreservingWorktree(true, null)).toBe(true);
  });
});
