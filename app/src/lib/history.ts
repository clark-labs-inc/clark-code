// Conversation history is CLOUD-ONLY (see lib/cloudHistory.ts — the source of
// truth). The desktop app no longer persists chats to localStorage, so a second
// account on the same machine can never see the first's chats and history
// follows the user across devices.
//
// This module keeps the small pure helpers the rest of the app still uses
// (ConversationMeta shape, title derivation, content check, run-settling of a
// persisted snapshot) plus `drainLocalHistory` — a one-time reader that lifts
// any chats left behind by prior local-first versions into memory so the store
// can upload them to the cloud, then deletes the local keys.

import type {
  ContentBlock,
  ResumeItem,
  ResumeTranscript,
  Snapshot,
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
  // Interrupted tool calls settle to "completed", not "failed": their chip
  // renders quietly (completion shows no glyph), whereas a red ✗ on every tool
  // that happened to be in flight when the session ended reads as an error the
  // user should act on. The run row above already says the run was cancelled.
  const tool_calls: Snapshot["tool_calls"] = {};
  for (const [id, t] of Object.entries(snapshot.tool_calls)) {
    if (t.status === "pending" || t.status === "in_progress") {
      tool_calls[id] = { ...t, status: "completed" };
      changed = true;
    } else {
      tool_calls[id] = t;
    }
  }
  if (!changed && !snapshot.pending_permission) return snapshot;
  return { ...snapshot, runs, tool_calls, pending_permission: undefined };
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
    }
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
        out.push({ meta, snapshot: JSON.parse(raw) as Snapshot, archived: !!meta.archived });
      } catch {
        /* skip a corrupt snapshot; its key is still removed */
      }
    }
  }

  for (const k of remove) store.removeItem(k);
  return out;
}
