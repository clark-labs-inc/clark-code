import type { ManagedWorktreeBase } from "../core-bridge/bridge";

const KEY = "agent-desktop:managed-worktree-base";

export function loadManagedWorktreeBase(): ManagedWorktreeBase {
  if (typeof localStorage === "undefined") return "current";
  return localStorage.getItem(KEY) === "default" ? "default" : "current";
}

export function saveManagedWorktreeBase(base: ManagedWorktreeBase): void {
  if (typeof localStorage !== "undefined") localStorage.setItem(KEY, base);
}
