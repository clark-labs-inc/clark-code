// Per-project shell-command allow/deny lists.

import { accountScopedKey } from "./accountProjectStorage";
//
// "Always allow this command" in the permission gate appends to a project's
// allowlist; the engine then skips the gate for matching Safe/Caution commands
// (and only those — see safety.rs). Persisted per project folder so trust
// doesn't leak between repos, and sent to the engine on connect.

const ALLOW_PREFIX = "clark-desktop:cmd-allow:";
const DENY_PREFIX = "clark-desktop:cmd-deny:";

function read(key: string): string[] {
  try {
    const raw = localStorage.getItem(key);
    const list = raw ? (JSON.parse(raw) as unknown) : [];
    return Array.isArray(list) ? list.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

function write(key: string, list: string[]): void {
  try {
    localStorage.setItem(key, JSON.stringify(list.slice(0, 200)));
  } catch {
    /* quota — best effort */
  }
}

const normalize = (cmd: string) => cmd.trim().replace(/\s+/g, " ");

export function loadAllowlist(project: string, scope?: string | null): string[] {
  return project ? read(accountScopedKey(ALLOW_PREFIX, scope) + project) : [];
}

export function loadDenylist(project: string, scope?: string | null): string[] {
  return project ? read(accountScopedKey(DENY_PREFIX, scope) + project) : [];
}

/** Append a command to a project's allowlist (idempotent). */
export function allowCommand(project: string, command: string, scope?: string | null): void {
  if (!project) return;
  const cmd = normalize(command);
  if (!cmd) return;
  const key = accountScopedKey(ALLOW_PREFIX, scope) + project;
  const list = read(key);
  if (!list.includes(cmd)) write(key, [...list, cmd]);
}

export function denyCommand(project: string, command: string, scope?: string | null): void {
  if (!project) return;
  const cmd = normalize(command);
  if (!cmd) return;
  const key = accountScopedKey(DENY_PREFIX, scope) + project;
  const list = read(key);
  if (!list.includes(cmd)) write(key, [...list, cmd]);
}

export function removeAllowed(project: string, command: string, scope?: string | null): void {
  if (!project) return;
  const key = accountScopedKey(ALLOW_PREFIX, scope) + project;
  write(key, read(key).filter((c) => c !== normalize(command)));
}

export function removeDenied(project: string, command: string, scope?: string | null): void {
  if (!project) return;
  const key = accountScopedKey(DENY_PREFIX, scope) + project;
  write(key, read(key).filter((c) => c !== normalize(command)));
}
