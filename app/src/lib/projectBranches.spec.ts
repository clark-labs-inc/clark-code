import { describe, expect, it } from "vitest";
import { resolveBranchSelection } from "./projectBranches";

describe("resolveBranchSelection", () => {
  it("opens the checkout that already owns the requested branch", () => {
    expect(
      resolveBranchSelection(
        { name: "main", checkoutPath: "/repos/project-main" },
        { cwd: "/repos/project", branch: "feature/local", detached: false },
      ),
    ).toEqual({ action: "open", path: "/repos/project-main" });
  });

  it("switches an unowned branch in the selected checkout", () => {
    expect(
      resolveBranchSelection(
        { name: "main", checkoutPath: null },
        { cwd: "/repos/project", branch: "feature/local", detached: false },
      ),
    ).toEqual({ action: "switch" });
  });

  it("recognizes the current branch without another Git operation", () => {
    expect(
      resolveBranchSelection(
        { name: "main", checkoutPath: "/repos/project/" },
        { cwd: "/repos/project", branch: "main", detached: false },
      ),
    ).toEqual({ action: "current" });
  });
});
