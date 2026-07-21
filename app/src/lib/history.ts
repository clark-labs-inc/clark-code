// Conversation history is CLOUD-AUTHORITATIVE (see lib/cloudHistory.ts). The
// Rust bridge may keep an account-scoped SQLite outbox and acknowledged cache
// for crash/offline recovery, but local disk never wins a revision conflict.
// The desktop app does not persist current chats to localStorage, so a second
// account on the same machine cannot see the first's chats and history follows
// the user across devices.
//
// This module keeps the small pure helpers the rest of the app still uses
// (ConversationMeta shape, title derivation, content check, run-settling of a
// persisted snapshot) plus `drainLocalHistory` — a one-time reader that lifts
// any chats left behind by prior local-first versions into memory so the store
// can upload them to the cloud, then deletes the local keys.

import {
  normalizeSnapshot,
  type WireSnapshot,
  type ContentBlock,
  type ExecutionChecklist,
  type ProposedPlan,
  type ResumeItem,
  type ResumeTranscript,
  type Snapshot,
  type TimelineItem,
} from "../core-bridge/types";

export interface ConversationMeta {
  /** Provider session/conversation id — the key used to resume. */
  id: string;
  /** Short title derived from the first user message. */
  title: string;
  provider: string;
  mode?: string;
  /** Absolute project folder this conversation ran in (local, or the remote
   *  project root for a remote session). */
  project?: string;
  /** When set, this conversation ran on a remote host (the SSH destination);
   *  reopening it re-establishes the tunnel. Distinguishes remote in the list. */
  remoteHost?: string;
  /** True once the user renamed it — auto-derived titles stop overwriting. */
  titleLocked?: boolean;
  createdAt: number;
  updatedAt: number;
  /** Server-owned snapshot revision. Local disk may cache it, but only a cloud
   * response advances it. */
  rev?: number;
  /** Soft-delete flag. Archived conversations are hidden from the main list
   *  (shown under a collapsed "Archived" section) but kept in the cloud so they
   *  can be restored with the full transcript. Now round-tripped through the
   *  cloud (desktop_conversation.archived), not local-only. */
  archived?: boolean;
}

function safeStore(): Storage | null {
  try {
    return typeof localStorage !== "undefined" ? localStorage : null;
  } catch {
    return null;
  }
}

/** A persisted transcript is never live: coerce any non-terminal run to a
 *  settled status, settle tool calls the same way, and drop a stale permission
 *  prompt, so a reopened (or reloaded) conversation never shows a stuck
 *  "Thinking…", a spinning tool chip, or a dead prompt. */
export function settleRuns(snapshot: Snapshot): Snapshot {
  snapshot = migratePlanningSnapshot(snapshot);
  let changed = false;
  const runs: Snapshot["runs"] = {};
  for (const [id, r] of Object.entries(snapshot.runs)) {
    if (r.status === "running" || r.status === "queued" || r.status === "awaiting_input") {
      runs[id] = { ...r, status: "cancelled" };
      changed = true;
    } else {
      runs[id] = r;
    }
  }
  // Preserve the fact that interrupted tool calls never completed. Treating
  // them as successful loses exactly the state a resumed model and the user
  // need in order to avoid trusting work that did not happen.
  const tool_calls: Snapshot["tool_calls"] = {};
  for (const [id, t] of Object.entries(snapshot.tool_calls)) {
    if (t.status === "pending" || t.status === "in_progress") {
      tool_calls[id] = { ...t, status: "cancelled" };
      changed = true;
    } else {
      tool_calls[id] = t;
    }
  }
  let goal = snapshot.goal;
  if (goal?.status === "active") {
    goal = {
      ...goal,
      status: "blocked",
      blocker_reason: "Clark stopped before the goal finished.",
    };
    changed = true;
  }
  const provider_incidents = Object.fromEntries(
    Object.entries(snapshot.provider_incidents).map(([id, incident]) => {
      if (incident.status !== "retrying" && incident.status !== "observed") {
        return [id, incident];
      }
      changed = true;
      return [id, { ...incident, status: "interrupted" as const }];
    }),
  );
  if (!changed && !snapshot.pending_permission) return snapshot;
  return { ...snapshot, runs, tool_calls, goal, provider_incidents, pending_permission: undefined };
}

/** Upgrade persisted snapshots from the original overloaded `plan` shape.
 * Cloud history is durable across app releases, so this boundary accepts the
 * old wire form once and returns only the current typed representation. */
export function migratePlanningSnapshot(snapshot: Snapshot): Snapshot {
  const legacy = snapshot as Snapshot & {
    plan?: { phases?: Array<{ title: string; status: string; priority?: string }> };
    timeline: Array<TimelineItem | {
      item: "plan";
      run?: string;
      plan?: { phases?: Array<{ title: string; status: string; priority?: string }> };
    }>;
  };
  const convert = (value: typeof legacy.plan): ExecutionChecklist | undefined => {
    if (!value?.phases) return undefined;
    return {
      revision: 0,
      steps: value.phases.map((phase) => ({
        title: phase.title,
        status: phase.status as ExecutionChecklist["steps"][number]["status"],
        ...(phase.priority ? { priority: phase.priority } : {}),
      })),
    };
  };
  const hasLegacyTimeline = legacy.timeline.some(
    (item) => (item as { item?: string }).item === "plan",
  );
  if (!legacy.plan && !hasLegacyTimeline) return snapshot;
  const timeline = legacy.timeline.map((item) => {
    const raw = item as TimelineItem | {
      item: "plan";
      run?: string;
      plan?: { phases?: Array<{ title: string; status: string; priority?: string }> };
    };
    if (raw.item !== "plan") return raw;
    return {
      item: "execution_checklist" as const,
      run: raw.run,
      checklist: convert(raw.plan),
    };
  });
  const execution_checklist = snapshot.execution_checklist ?? convert(legacy.plan);
  const migrated = { ...snapshot, timeline, execution_checklist } as Snapshot & { plan?: unknown };
  delete migrated.plan;
  return migrated;
}

function replayBlocks(blocks: ContentBlock[]): ContentBlock[] {
  return blocks.filter((block) => block.type !== "thinking");
}

function trimItemToBudget(item: ResumeItem, budget: number): ResumeItem {
  const suffix = (blocks: ContentBlock[]) => {
    const text = blocks
      .map((block) => (block.type === "text" ? block.text : `[${block.type}]`))
      .join("");
    return [{ type: "text" as const, text: `…${text.slice(-Math.max(0, budget - 200))}` }];
  };
  if (item.item === "goal" || item.item === "proposed_plan") return item;
  if (item.item === "message") return { ...item, blocks: suffix(item.blocks) };
  return { ...item, arguments: undefined, content: suffix(item.content) };
}

/** Build typed provider history for a reopened conversation. Recent items are
 * retained under a bounded serialized budget; tool arguments/results stay
 * structured instead of being flattened into ambiguous system-prompt prose. */
export function buildResumeTranscript(
  snapshot: Snapshot,
  budget = 24000,
): ResumeTranscript | null {
  const items: ResumeItem[] = [];
  for (const item of snapshot.timeline) {
    if (item.item === "message") {
      const blocks = replayBlocks(item.blocks).map((block) =>
        block.type === "text"
          ? { ...block, text: block.text.replace(/<thinking>[\s\S]*?(<\/thinking>|$)/g, "") }
          : block,
      );
      if (blocks.length > 0) items.push({ item: "message", role: item.role, blocks });
    } else if (item.item === "tool_call") {
      const tool = snapshot.tool_calls[item.id];
      if (!tool) continue;
      items.push({
        item: "tool_call",
        id: tool.id,
        tool_name: tool.tool_name,
        title: tool.title,
        kind: tool.kind,
        status: tool.status,
        locations: tool.locations,
        arguments: tool.raw_input,
        content: replayBlocks(tool.content),
      });
    } else if (item.item === "proposed_plan") {
      items.push({ item: "proposed_plan", plan: item.plan });
    }
  }
  if (snapshot.goal) items.push({ item: "goal", goal: snapshot.goal });
  if (
    snapshot.proposed_plan &&
    !items.some((item) => item.item === "proposed_plan" && item.plan.id === snapshot.proposed_plan?.id)
  ) {
    items.push({ item: "proposed_plan", plan: snapshot.proposed_plan });
  }
  if (items.length === 0) return null;

  const kept: ResumeItem[] = [];
  let remaining = budget;
  let truncated = false;
  for (let index = items.length - 1; index >= 0; index--) {
    const item = items[index];
    const cost = JSON.stringify(item).length;
    if (cost <= remaining) {
      kept.push(item);
      remaining -= cost;
      continue;
    }
    truncated = true;
    if (kept.length === 0 && remaining > 256) {
      kept.push(trimItemToBudget(item, remaining));
    }
    break;
  }
  kept.reverse();
  return { items: kept, truncated: truncated || kept.length < items.length };
}

/** Return the settled conversation prefix before one timeline item. Used by
 * edit-and-resend: everything from the selected user turn onward belongs to
 * the abandoned branch and must be removed from both display and replay. */
export function snapshotBeforeTimelineItem(snapshot: Snapshot, index: number): Snapshot {
  const timeline = snapshot.timeline.slice(0, Math.max(0, index));
  const toolIds = new Set(
    timeline.flatMap((item) => (item.item === "tool_call" ? [item.id] : [])),
  );
  const artifactIds = new Set(
    timeline.flatMap((item) => (item.item === "artifact" ? [item.id] : [])),
  );
  const runIds = new Set(
    timeline.flatMap((item) => {
      if (item.item === "message") return [item.run];
      if (item.item === "tool_call" && item.run) return [item.run];
      if (item.item === "execution_checklist" && item.run) return [item.run];
      if (item.item === "proposed_plan") return [item.run];
      if (item.item === "provider_incident") return [item.run];
      return [];
    }),
  );
  const runs = Object.fromEntries(
    Object.entries(snapshot.runs).filter(([id]) => runIds.has(id)),
  );
  const tool_calls = Object.fromEntries(
    Object.entries(snapshot.tool_calls).filter(([id]) => toolIds.has(id)),
  );
  const artifacts = snapshot.artifacts.filter((artifact) => artifactIds.has(artifact.id));
  const incidentIds = new Set(
    timeline.flatMap((item) => (item.item === "provider_incident" ? [item.id] : [])),
  );
  const provider_incidents = Object.fromEntries(
    Object.entries(snapshot.provider_incidents).filter(([id]) => incidentIds.has(id)),
  );
  let lastChecklist: ExecutionChecklist | undefined;
  let lastProposedPlan: ProposedPlan | undefined;
  for (const item of timeline) {
    if (item.item === "execution_checklist" && item.checklist) lastChecklist = item.checklist;
    if (item.item === "proposed_plan") lastProposedPlan = item.plan;
  }

  return {
    ...(snapshot.session ? { session: snapshot.session } : {}),
    runs,
    timeline,
    tool_calls,
    ...(lastChecklist ? { execution_checklist: lastChecklist } : {}),
    ...(lastProposedPlan ? { proposed_plan: lastProposedPlan } : {}),
    ...(snapshot.goal?.run && runIds.has(snapshot.goal.run) ? { goal: snapshot.goal } : {}),
    artifacts,
    provider_incidents,
  };
}

/** First user message → a compact title; falls back to a generic label. */
export function deriveTitle(snapshot: Snapshot): string {
  for (const item of snapshot.timeline) {
    if (item.item === "message" && item.role === "user") {
      const text = item.blocks
        .map((b) => (b.type === "text" ? b.text : ""))
        .join(" ")
        .trim()
        .replace(/\s+/g, " ");
      if (text) return text.length > 60 ? text.slice(0, 57) + "…" : text;
    }
  }
  return "New conversation";
}

/** True once a conversation has any user/agent message worth persisting. */
export function hasContent(snapshot: Snapshot): boolean {
  return snapshot.timeline.some((t) => t.item === "message");
}

// --- One-time migration off localStorage ---------------------------------

export interface DrainedConversation {
  meta: ConversationMeta;
  snapshot: Snapshot;
  archived: boolean;
}

/** Read every conversation left in localStorage by prior (local-first) versions
 *  — the old global `clark.history.*` keys AND the per-account scoped keys from
 *  v0.1.19 — return them so the store can upload them to the cloud, and delete
 *  every one of those keys. Self-cleaning: once drained, a later launch finds
 *  nothing to migrate. Preferences and the auth session use different keys and
 *  are untouched. */
export function drainLocalHistory(): DrainedConversation[] {
  const store = safeStore();
  if (!store) return [];

  // Index keys: "clark.history.index.v1" (legacy global) or
  // "clark.history.index.v1.<scope>" (per-account, v0.1.19).
  const INDEX_RE = /^clark\.history\.index\.v1(?:\.(.+))?$/;
  const indexKeys: string[] = [];
  for (let i = 0; i < store.length; i++) {
    const k = store.key(i);
    if (k && INDEX_RE.test(k)) indexKeys.push(k);
  }

  const out: DrainedConversation[] = [];
  const remove: string[] = [];

  for (const idxKey of indexKeys) {
    remove.push(idxKey);
    const scopeSuffix = idxKey.match(INDEX_RE)?.[1] ? `${idxKey.match(INDEX_RE)![1]}.` : "";
    let list: ConversationMeta[] = [];
    try {
      list = JSON.parse(store.getItem(idxKey) || "[]") as ConversationMeta[];
    } catch {
      list = [];
    }
    for (const meta of list) {
      const snapKey = `clark.history.snap.v1.${scopeSuffix}${meta.id}`;
      remove.push(snapKey);
      const raw = store.getItem(snapKey);
      if (!raw) continue;
      try {
        out.push({
          meta,
          snapshot: normalizeSnapshot(JSON.parse(raw) as WireSnapshot),
          archived: !!meta.archived,
        });
      } catch {
        /* skip a corrupt snapshot; its key is still removed */
      }
    }
  }

  for (const k of remove) store.removeItem(k);
  return out;
}
