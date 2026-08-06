import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { ProjectWorktreeTransitionPlan } from "../core-bridge/bridge";
import { ManagedWorktreeTransitionContent } from "./ManagedWorktreeJourney";

function plan(
  overrides: Partial<ProjectWorktreeTransitionPlan> = {},
): ProjectWorktreeTransitionPlan {
  return {
    sourceRoot: "/repo/clark-desktop",
    sourceBranch: "feature/current-work",
    sourceRevision: "1111111111111111111111111111111111111111",
    sourceChanges: { changedFiles: 2, untrackedFiles: 1, conflictedFiles: 0 },
    sourceIsManaged: false,
    targetBranch: null,
    targetCheckoutPath: null,
    action: "create_isolated",
    preservation: "changes_remain_in_source",
    requiresConfirmation: true,
    baseOptions: [
      {
        id: "current",
        label: "Current checkout (feature/current-work)",
        reference: "feature/current-work",
        revision: "1111111111111111111111111111111111111111",
        fallback: false,
      },
      {
        id: "default",
        label: "Default branch (origin/main)",
        reference: "origin/main",
        revision: "2222222222222222222222222222222222222222",
        fallback: false,
      },
    ],
    managedLocation: "/repo/clark-desktop.clark-worktrees",
    ...overrides,
  };
}

describe("managed worktree decision copy", () => {
  it("presents dirty-chat choices as concrete destinations", () => {
    const markup = renderToStaticMarkup(
      <ManagedWorktreeTransitionContent
        plan={plan()}
        base="current"
        setBase={vi.fn()}
        confirm={vi.fn()}
        dismiss={vi.fn()}
        preparing={false}
      />,
    );

    expect(markup).toContain("Where should this chat work?");
    expect(markup).toContain("feature/current-work · stays here");
    expect(markup).toContain("Work in this checkout");
    expect(markup).toContain("Create separate worktree");
    expect(markup).not.toContain("111111111111");
    expect(markup).not.toContain("legacy detached work");
  });

  it("makes a branch selection a branch-specific open-or-cancel choice", () => {
    const markup = renderToStaticMarkup(
      <ManagedWorktreeTransitionContent
        plan={plan({ action: "preserve_changes", targetBranch: "feature/target" })}
        base="current"
        setBase={vi.fn()}
        confirm={vi.fn()}
        dismiss={vi.fn()}
        preparing={false}
      />,
    );

    expect(markup).toContain("Open feature/target without moving your files?");
    expect(markup).toContain("feature/target · new worktree");
    expect(markup).toContain("Cancel branch change");
    expect(markup).toContain("Open feature/target");
    expect(markup).not.toContain("Work in this checkout");
  });
});
