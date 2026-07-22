import type { ProjectBranch } from "../core-bridge/bridge";

export type BranchSelection =
  | { action: "current" }
  | { action: "open"; path: string }
  | { action: "switch" };

function normalizedPath(path: string): string {
  return path.replaceAll("\\", "/").replace(/\/+$/, "");
}

export function resolveBranchSelection(
  branch: ProjectBranch,
  checkout: { cwd: string; branch: string; detached: boolean },
): BranchSelection {
  if (!checkout.detached && branch.name === checkout.branch) {
    return { action: "current" };
  }
  if (
    branch.checkoutPath
    && normalizedPath(branch.checkoutPath) !== normalizedPath(checkout.cwd)
  ) {
    return { action: "open", path: branch.checkoutPath };
  }
  return { action: "switch" };
}
