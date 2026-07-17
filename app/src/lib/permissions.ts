// Codex-style permission policy for how agent actions get approved. The mode is
// persisted locally and applied in the session store's snapshot handler: under
// "full" every request is auto-granted, under "auto" only non-destructive ones,
// and under "ask" the inline PermissionGate prompts the user.

import type { PermissionOption, PermissionRequest } from "../core-bridge/types";

export type PermissionMode = "ask" | "auto" | "full" | "plan";

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
  { id: "plan", label: "Plan first", description: "Research read-only, then propose a plan before any edits" },
];

/** Shift+Tab cycle order — mirrors Claude Code's mode-cycling shortcut. */
const MODE_CYCLE: PermissionMode[] = ["ask", "auto", "full", "plan"];

export function nextPermissionMode(mode: PermissionMode): PermissionMode {
  const i = MODE_CYCLE.indexOf(mode);
  return MODE_CYCLE[(i + 1) % MODE_CYCLE.length];
}

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
  // A plan decision (approving one, or entering plan mode) always needs an
  // explicit human answer, in every mode — the engine forces `ask` for these
  // server-side regardless of session policy.
  if (req.risk === "plan" || req.risk === "plan_entry") return false;
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
    if (v === "ask" || v === "auto" || v === "full" || v === "plan") return v;
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
