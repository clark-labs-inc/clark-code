import { beforeEach, describe, expect, it } from "vitest";
import { loadManagedWorktreeBase, saveManagedWorktreeBase } from "./managedWorktreeSettings";

describe("managed worktree starting-point settings", () => {
  beforeEach(() => localStorage.clear());

  it("does not leak a default-branch choice across projects or accounts", () => {
    saveManagedWorktreeBase("default", "id:stan", "/repo/one");

    expect(loadManagedWorktreeBase("id:stan", "/repo/one")).toBe("default");
    expect(loadManagedWorktreeBase("id:stan", "/repo/two")).toBe("current");
    expect(loadManagedWorktreeBase("id:other", "/repo/one")).toBe("current");
  });

  it("does not inherit the old unscoped setting", () => {
    localStorage.setItem("agent-desktop:managed-worktree-base", "default");
    expect(loadManagedWorktreeBase("id:stan", "/repo/one")).toBe("current");
  });
});
