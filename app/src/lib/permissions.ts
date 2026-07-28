// Approval policy and collaboration mode are orthogonal preferences. Approval
// answers "which actions need confirmation?"; collaboration answers "are we
// executing or agreeing on a read-only plan?".

import type {
  CollaborationMode,
  PermissionOption,
  PermissionRequest,
} from "../core-bridge/types";

export type ApprovalPolicy = "ask" | "auto" | "full";

export interface ApprovalPolicyInfo {
  id: ApprovalPolicy;
  label: string;
  description: string;
}

/** Order matters: shown top-to-bottom in the picker. */
export const APPROVAL_POLICIES: ApprovalPolicyInfo[] = [
  {
    id: "ask",
    label: "Ask for approval",
    description: "Review edits, commands, and connected actions before they run",
  },
  {
    id: "auto",
    label: "Approve for me",
    description: "Run routine project actions; ask at risky, network, and external boundaries",
  },
  {
    id: "full",
    label: "Full access",
    description: "Run directly on your machine without Clark’s command sandbox or action approvals",
  },
];

const APPROVAL_CYCLE: ApprovalPolicy[] = ["ask", "auto", "full"];

export function nextApprovalPolicy(policy: ApprovalPolicy): ApprovalPolicy {
  const index = APPROVAL_CYCLE.indexOf(policy);
  return APPROVAL_CYCLE[(index + 1) % APPROVAL_CYCLE.length];
}

export const DEFAULT_APPROVAL_POLICY: ApprovalPolicy = "auto";
export const DEFAULT_COLLABORATION_MODE: CollaborationMode = "default";

/** The option that grants the request, preferring a one-time allow. */
export function pickAllowOption(req: PermissionRequest): PermissionOption | undefined {
  return (
    req.options.find((option) => option.kind === "allow_once") ??
    req.options.find((option) => option.kind === "allow_always") ??
    req.options.find((option) => option.kind.startsWith("allow"))
  );
}

/** Whether an approval policy grants this request without prompting. */
export function wouldAutoApprove(policy: ApprovalPolicy, req: PermissionRequest): boolean {
  if (!pickAllowOption(req)) return false;
  // Entering Plan Mode is a collaboration choice, not an action approval.
  if (req.risk === "plan_entry") return false;
  // Backend confirmation gates exist precisely to get a human answer.
  if (req.risk === "confirm") return false;
  // Full access covers the complete action surface: local mutations, shell,
  // websites, MCP/external tools, and billed generation. Collaboration choices
  // and explicit backend confirmations above are not action permissions.
  if (policy === "full") return true;
  if (policy === "auto") {
    return (
      req.risk !== "caution" &&
      req.risk !== "danger" &&
      req.risk !== "network" &&
      req.risk !== "sandbox" &&
      req.risk !== "external" &&
      req.risk !== "billed"
    );
  }
  return false;
}

const APPROVAL_KEY = "clark-desktop:approval-policy";
const COLLABORATION_KEY = "clark-desktop:collaboration-mode";
const LEGACY_MODE_KEY = "clark-desktop:permission-mode";

function legacyMode(): string | null {
  try {
    return localStorage.getItem(LEGACY_MODE_KEY);
  } catch {
    return null;
  }
}

export function loadApprovalPolicy(): ApprovalPolicy {
  try {
    const value = localStorage.getItem(APPROVAL_KEY);
    if (value === "ask" || value === "auto" || value === "full") return value;
  } catch {
    return DEFAULT_APPROVAL_POLICY;
  }
  const legacy = legacyMode();
  return legacy === "ask" || legacy === "full" ? legacy : DEFAULT_APPROVAL_POLICY;
}

export function saveApprovalPolicy(policy: ApprovalPolicy): void {
  try {
    localStorage.setItem(APPROVAL_KEY, policy);
  } catch {
    /* ignore */
  }
}

export function loadCollaborationMode(): CollaborationMode {
  try {
    const value = localStorage.getItem(COLLABORATION_KEY);
    if (value === "default" || value === "plan") return value;
  } catch {
    return DEFAULT_COLLABORATION_MODE;
  }
  return legacyMode() === "plan" ? "plan" : DEFAULT_COLLABORATION_MODE;
}

export function saveCollaborationMode(mode: CollaborationMode): void {
  try {
    localStorage.setItem(COLLABORATION_KEY, mode);
  } catch {
    /* ignore */
  }
}
