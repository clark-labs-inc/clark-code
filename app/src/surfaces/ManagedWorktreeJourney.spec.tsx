import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ProjectWorktreeTransitionPlan } from "../core-bridge/bridge";
import { useSessionStore } from "../store/sessionStore";
import { useSpecialistStore } from "../store/specialistStore";
import {
  ManagedWorktreeBasePicker,
  ManagedWorktreeTransitionContent,
} from "./ManagedWorktreeJourney";

function plan(
  overrides: Partial<ProjectWorktreeTransitionPlan> = {},
): ProjectWorktreeTransitionPlan {
  return {
    sourceRoot: "/repo/example-desktop",
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
    managedLocation: "/repo/example-desktop.agent-worktrees",
    ...overrides,
  };
}

describe("managed worktree decision copy", () => {
  beforeEach(() => {
    localStorage.clear();
    useSpecialistStore.getState().close();
    useSessionStore.setState({
      auth: null,
      localSettings: { ...useSessionStore.getState().localSettings, cwd: "/repo/example-desktop" },
      managedWorktreeBase: "current",
    });
  });

  it("labels the selected checkout as the default new-chat destination", () => {
    useSessionStore.setState({ managedWorktreeBase: "current" });

    const markup = renderToStaticMarkup(<ManagedWorktreeBasePicker />);

    expect(markup).toContain("New chat · This checkout");
    expect(markup).toContain("New chat starts in this checkout");
  });

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

  it("restores an independent starting point when switching specialists", () => {
    // The neutral foundation test product has no branded catalog, so drive the
    // store boundary directly with the same active identities a product emits.
    useSpecialistStore.setState({ active: "rsi" });
    useSessionStore.getState().setManagedWorktreeBase("default");

    useSpecialistStore.setState({ active: "scientist" });
    expect(useSessionStore.getState().managedWorktreeBase).toBe("current");
    useSessionStore.getState().setManagedWorktreeBase("current");

    useSpecialistStore.setState({ active: "rsi" });
    expect(useSessionStore.getState().managedWorktreeBase).toBe("default");

    useSpecialistStore.getState().close();
    expect(useSessionStore.getState().managedWorktreeBase).toBe("current");
  });
});
