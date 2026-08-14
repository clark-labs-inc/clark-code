// Approval policy and collaboration mode are orthogonal preferences. Approval
// answers "which actions need confirmation?"; collaboration answers "are we
// executing or agreeing on a read-only plan?".

import type {
  CollaborationMode,
  PermissionOption,
  PermissionRequest,
} from "../core-bridge/types";
import { accountScopedKey } from "./accountProjectStorage";

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
    description: "Run directly on your machine without the agent’s command sandbox or action approvals",
  },
];

const APPROVAL_CYCLE: ApprovalPolicy[] = ["ask", "auto", "full"];

export function nextApprovalPolicy(policy: ApprovalPolicy): ApprovalPolicy {
  const index = APPROVAL_CYCLE.indexOf(policy);
  return APPROVAL_CYCLE[(index + 1) % APPROVAL_CYCLE.length];
}

export const DEFAULT_APPROVAL_POLICY: ApprovalPolicy = "auto";
export const DEFAULT_COLLABORATION_MODE: CollaborationMode = "default";

/** Scout owns an organization-wide cartography run rather than a bounded
 * project checkout, so its execution authority is a product invariant rather
 * than a user-selectable conversation preference. */
export function approvalPolicyForSpecialist(
  policy: ApprovalPolicy,
  specialistKind?: string | null,
): ApprovalPolicy {
  return specialistKind === "scout" ? "full" : policy;
}

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

const APPROVAL_KEY = "agent-desktop:approval-policy";
const APPROVAL_POLICIES_KEY = "agent-desktop:approval-policies";
const COLLABORATION_KEY = "agent-desktop:collaboration-mode";
const LEGACY_MODE_KEY = "agent-desktop:permission-mode";

function legacyMode(): string | null {
  try {
    return localStorage.getItem(LEGACY_MODE_KEY);
  } catch {
    return null;
  }
}

export function loadApprovalPolicy(scope?: string | null): ApprovalPolicy {
  try {
    const value = localStorage.getItem(accountScopedKey(APPROVAL_KEY, scope));
    if (value === "ask" || value === "auto" || value === "full") return value;
  } catch {
    return DEFAULT_APPROVAL_POLICY;
  }
  const legacy = scope === undefined ? legacyMode() : null;
  return legacy === "ask" || legacy === "full" ? legacy : DEFAULT_APPROVAL_POLICY;
}

export function saveApprovalPolicy(policy: ApprovalPolicy, scope?: string | null): void {
  try {
    localStorage.setItem(accountScopedKey(APPROVAL_KEY, scope), policy);
  } catch {
    /* ignore */
  }
}

/** Only the three known policies are ever stored; anything else is dropped. */
function normalizeApprovalPolicy(value: unknown): ApprovalPolicy | undefined {
  return value === "ask" || value === "auto" || value === "full" ? value : undefined;
}

/** Per-conversation approval-policy overrides, keyed by conversation id. A chat
 *  with no entry falls back to the account's global default (`approvalPolicy`).
 *  Mirrors `chatModels`: each chat keeps its own level, persisted here rather
 *  than the single global key so switching chats never edits what others run. */
export function loadApprovalPolicies(scope?: string | null): Record<string, ApprovalPolicy> {
  try {
    const key = accountScopedKey(APPROVAL_POLICIES_KEY, scope);
    const raw = localStorage.getItem(key);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    const policies: Record<string, ApprovalPolicy> = {};
    for (const [id, value] of Object.entries(parsed as Record<string, unknown>)) {
      const policy = normalizeApprovalPolicy(value);
      if (policy) policies[id] = policy;
    }
    const normalized = JSON.stringify(policies);
    if (normalized !== raw) localStorage.setItem(key, normalized);
    return policies;
  } catch {
    return {};
  }
}

export function saveApprovalPolicies(
  policies: Record<string, ApprovalPolicy>,
  scope?: string | null,
): void {
  try {
    const cleaned: Record<string, ApprovalPolicy> = {};
    for (const [id, value] of Object.entries(policies)) {
      const policy = normalizeApprovalPolicy(value);
      if (policy) cleaned[id] = policy;
    }
    localStorage.setItem(accountScopedKey(APPROVAL_POLICIES_KEY, scope), JSON.stringify(cleaned));
  } catch {
    /* Non-fatal, mirroring saveApprovalPolicy. */
  }
}

export function loadCollaborationMode(scope?: string | null): CollaborationMode {
  try {
    const value = localStorage.getItem(accountScopedKey(COLLABORATION_KEY, scope));
    if (value === "default" || value === "plan") return value;
  } catch {
    return DEFAULT_COLLABORATION_MODE;
  }
  return scope === undefined && legacyMode() === "plan" ? "plan" : DEFAULT_COLLABORATION_MODE;
}

export function saveCollaborationMode(mode: CollaborationMode, scope?: string | null): void {
  try {
    localStorage.setItem(accountScopedKey(COLLABORATION_KEY, scope), mode);
  } catch {
    /* ignore */
  }
}
