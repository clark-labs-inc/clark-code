import type { ManagedWorktreeBase } from "../core-bridge/bridge";

const KEY = "agent-desktop:managed-worktree-base";
const ANONYMOUS_SCOPE = "anonymous";

function scopedKey(accountScope: string | null | undefined, projectPath: string | null | undefined): string {
  const account = accountScope?.trim().toLowerCase() || ANONYMOUS_SCOPE;
  const project = projectPath?.replaceAll("\\", "/").replace(/\/+$/, "").trim();
  return `${KEY}:${encodeURIComponent(account)}:${encodeURIComponent(project || "no-project")}`;
}

export function loadManagedWorktreeBase(
  accountScope?: string | null,
  projectPath?: string | null,
): ManagedWorktreeBase {
  if (typeof localStorage === "undefined") return "current";
  return localStorage.getItem(scopedKey(accountScope, projectPath)) === "default" ? "default" : "current";
}

export function saveManagedWorktreeBase(
  base: ManagedWorktreeBase,
  accountScope?: string | null,
  projectPath?: string | null,
): void {
  if (typeof localStorage !== "undefined") {
    localStorage.setItem(scopedKey(accountScope, projectPath), base);
  }
}
