import { describe, expect, it } from "vitest";
import {
  FakeGitRepository,
  FAKE_GIT_SIMULATION_STORAGE_KEY,
  fakeGitScenario,
  fakeGitChanges,
} from "./fakeGitRepository";

describe("FakeGitRepository", () => {
  it("lets browser previews select a dirty scenario from the URL", () => {
    const globalScope = globalThis as typeof globalThis & {
      window?: { location?: { search?: string } };
    };
    const previousWindow = globalScope.window;
    localStorage.setItem(FAKE_GIT_SIMULATION_STORAGE_KEY, "clean");
    Object.defineProperty(globalScope, "window", {
      configurable: true,
      value: { location: { search: "?fakeGit=modified" } },
    });
    try {
      expect(fakeGitScenario()).toBe("modified");
    } finally {
      if (previousWindow) {
        Object.defineProperty(globalScope, "window", { configurable: true, value: previousWindow });
      } else {
        Reflect.deleteProperty(globalScope, "window");
      }
      localStorage.clear();
    }
  });

  it("keeps dirty tracked, untracked, and conflicted work in place", () => {
    for (const scenario of ["modified", "untracked", "conflicted"] as const) {
      const git = new FakeGitRepository("/repo", scenario);
      expect(() => git.switchBranch("/repo", "feature/checkout-context")).toThrow(
        "Commit or remove local changes before switching branches.",
      );
      expect(git.context("/repo")?.activity).toMatchObject(fakeGitChanges(scenario));
      expect(git.plan("/repo", "feature/checkout-context")).toMatchObject({
        action: "preserve_changes",
        preservation: "changes_remain_in_source",
        requiresConfirmation: true,
      });
    }
  });

  it("routes an owned branch to its checkout and never nests managed worktrees", () => {
    const git = new FakeGitRepository("/repo");
    const managed = git.createManaged("/repo", { base: "current" });
    expect(git.plan("/repo", `clark/${managed.id}`)).toMatchObject({
      action: "open_owner",
      targetCheckoutPath: managed.path,
      preservation: "owner_checkout",
    });
    expect(() => git.createManaged(managed.path, { base: "current" })).toThrow(
      "already a Clark-managed isolated worktree",
    );
    expect(() => git.plan(managed.path, "feature/checkout-context")).toThrow("pinned");
  });

  it("requires save-before-cleanup for committed managed work", () => {
    const git = new FakeGitRepository("/repo");
    const managed = git.createManaged("/repo", { base: "current" }, "committed");
    expect(() => git.cleanupManaged("/repo", managed.id)).toThrow("not protected by a branch");
    const saved = git.saveManaged("/repo", managed.id);
    expect(saved.branch).toBe(`clark/${managed.id}-saved`);
    expect(git.cleanupManaged("/repo", managed.id).removed).toBe(true);
  });
});
