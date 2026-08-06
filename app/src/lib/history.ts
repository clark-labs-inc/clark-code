// Conversation history is CLOUD-AUTHORITATIVE (see lib/cloudHistory.ts). The
// Rust bridge may keep an account-scoped SQLite outbox and acknowledged cache
// for crash/offline recovery, but local disk never wins a revision conflict.
// The desktop app does not persist current chats to localStorage, so a second
// account on the same machine cannot see the first's chats and history follows
// the user across devices.
//
// This module keeps only the pure helpers used by current cloud/native history.

import {
  type ContentBlock,
  type ExecutionChecklist,
  type ProposedPlan,
  type ResumeItem,
  type ResumeTranscript,
  type Snapshot,
  type TimelineItem,
} from "../core-bridge/types";
import type { SpecialistContext } from "./specialists";

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
  /** Specialist workspace binding restored with this cloud conversation. */
  specialist?: SpecialistContext;
}

/** Settle an explicitly abandoned in-memory snapshot. Do not use this for
 * cloud reads: another desktop may still own that live run, and the service
 * trajectory status is the authoritative lifecycle boundary. */
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
    plan?: unknown;
    timeline?: Array<TimelineItem | {
      item: "plan";
      run?: string;
      plan?: unknown;
      explanation?: string;
    }>;
  };
  const convert = (value: unknown): ExecutionChecklist | undefined => {
    if (!value || typeof value !== "object") return undefined;
    const phases = (value as { phases?: unknown }).phases;
    if (!Array.isArray(phases)) return undefined;
    const revision = (value as { revision?: unknown }).revision;
    return {
      revision: typeof revision === "number" && Number.isFinite(revision)
        ? Math.max(0, Math.floor(revision))
        : 0,
      steps: phases.flatMap((phase) => {
        if (!phase || typeof phase !== "object") return [];
        const title = (phase as { title?: unknown }).title;
        if (typeof title !== "string" || title.trim().length === 0) return [];
        const rawStatus = String((phase as { status?: unknown }).status ?? "pending")
          .toLowerCase();
        const status: ExecutionChecklist["steps"][number]["status"] =
          rawStatus === "in_progress" || rawStatus === "in-progress" || rawStatus === "running"
            ? "in_progress"
            : rawStatus === "completed" || rawStatus === "complete" || rawStatus === "done"
              ? "completed"
              : "pending";
        const priority = (phase as { priority?: unknown }).priority;
        return [{
          title: title.trim(),
          status,
          ...(typeof priority === "string" && priority.trim().length > 0
            ? { priority: priority.trim() }
            : {}),
        }];
      }),
    };
  };
  const timeline = Array.isArray(legacy.timeline) ? legacy.timeline : [];
  const hasLegacyTimeline = timeline.some(
    (item) => (item as { item?: string }).item === "plan",
  );
  if (legacy.plan === undefined && !hasLegacyTimeline) return snapshot;
  let latestLegacyChecklist = convert(legacy.plan);
  const migratedTimeline = timeline.map((item) => {
    const raw = item as TimelineItem | {
      item: "plan";
      run?: string;
      plan?: unknown;
      explanation?: string;
    };
    if (raw.item !== "plan") return raw;
    const checklist = convert(raw.plan) ?? latestLegacyChecklist ?? { revision: 0, steps: [] };
    latestLegacyChecklist = checklist;
    return {
      item: "execution_checklist" as const,
      run: raw.run,
      checklist,
      ...(raw.explanation ? { explanation: raw.explanation } : {}),
    };
  });
  const execution_checklist = snapshot.execution_checklist ?? latestLegacyChecklist;
  const migrated = {
    ...snapshot,
    timeline: migratedTimeline,
    ...(execution_checklist ? { execution_checklist } : {}),
  } as Snapshot & { plan?: unknown };
  delete migrated.plan;
  return migrated;
}

function replayBlocks(blocks: ContentBlock[]): ContentBlock[] {
  return blocks.map((block) => ({ ...block }));
}

/** Build complete typed provider history for a reopened conversation. A model
 * checkpoint already represents intentional compaction; this replay boundary
 * must not apply a second, independent tail window. */
export function buildResumeTranscript(snapshot: Snapshot): ResumeTranscript | null {
  const checkpoint = snapshot.model_context_checkpoint;
  const checkpointItems = checkpoint?.transcript.items ?? [];
  const items: ResumeItem[] = [];
  for (const item of snapshot.timeline.slice(checkpoint?.timeline_index ?? 0)) {
    if (item.item === "message") {
      const blocks = replayBlocks(item.blocks);
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
  if (checkpointItems.length === 0 && items.length === 0) return null;

  return {
    items: [...checkpointItems, ...items],
    truncated: checkpoint?.transcript.truncated ?? false,
  };
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
    ...(snapshot.model_context_checkpoint &&
    index >= snapshot.model_context_checkpoint.timeline_index
      ? { model_context_checkpoint: snapshot.model_context_checkpoint }
      : {}),
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
