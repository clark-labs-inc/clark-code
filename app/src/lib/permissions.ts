// Codex-style permission policy for how agent actions get approved. The mode is
// persisted locally and applied in the session store's snapshot handler: under
// "full" every request is auto-granted, under "auto" only non-destructive ones,
// and under "ask" the inline PermissionGate prompts the user.

import type { PermissionOption, PermissionRequest } from "../core-bridge/types";

export type PermissionMode = "ask" | "auto" | "full";

export interface PermissionModeInfo {
  id: PermissionMode;
  label: string;
  description: string;
}

/** Order matters: shown top-to-bottom in the picker. */
export const PERMISSION_MODES: PermissionModeInfo[] = [
  { id: "ask", label: "Ask for approval", description: "Review every edit and command before it runs" },
  { id: "auto", label: "Approve for me", description: "Auto-run safe edits & commands; ask before anything destructive" },
  { id: "full", label: "Full access", description: "Run everything without asking (catastrophic commands are still blocked)" },
];

/** Safe by default: auto-run safe work, prompt on anything the engine classifies
 *  destructive. (Catastrophic commands are refused engine-side regardless.) */
export const DEFAULT_PERMISSION_MODE: PermissionMode = "auto";

/** The option that grants the request, preferring a one-time allow. */
export function pickAllowOption(req: PermissionRequest): PermissionOption | undefined {
  return (
    req.options.find((o) => o.kind === "allow_once") ??
    req.options.find((o) => o.kind === "allow_always") ??
    req.options.find((o) => o.kind.startsWith("allow"))
  );
}

/** Whether the current mode grants this request without prompting. The engine
 *  classifies shell commands and sends an authoritative `risk`; it has already
 *  refused anything catastrophic, so nothing here can run a blocked command. */
export function wouldAutoApprove(mode: PermissionMode, req: PermissionRequest): boolean {
  if (!pickAllowOption(req)) return false; // nothing to grant — must prompt
  if (mode === "full") return true;
  // Ask only for destructive shell commands and external (MCP) tools on first
  // use; everything else (reads, sandboxed edits, safe/caution shell) auto-runs.
  if (mode === "auto") return req.risk !== "danger" && req.risk !== "external";
  return false; // "ask" — always prompt
}

const KEY = "clark-desktop:permission-mode";

export function loadPermissionMode(): PermissionMode {
  try {
    const v = localStorage.getItem(KEY);
    if (v === "ask" || v === "auto" || v === "full") return v;
  } catch {
    /* ignore */
  }
  return DEFAULT_PERMISSION_MODE;
}

export function savePermissionMode(mode: PermissionMode): void {
  try {
    localStorage.setItem(KEY, mode);
  } catch {
    /* ignore */
  }
}
