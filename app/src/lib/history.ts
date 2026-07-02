// Local conversation history.
//
// Persists a small index of past conversations plus each one's last Snapshot, so
// the UI can list prior chats and reopen them. Storage is intentionally behind a
// thin seam: today it uses `localStorage` (which the Tauri WebView persists
// per-origin, and which works in browser preview too); a future swap to the
// Tauri fs/app-data dir is a one-file change that keeps this module's API.

import type { Snapshot } from "../core-bridge/types";

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
   *  (shown under a collapsed "Archived" section) but never removed locally or
   *  from the cloud, so they can be restored with the full transcript + artifacts.
   *  Local-only for now; not round-tripped through the cloud summary. */
  archived?: boolean;
}

const INDEX_KEY = "clark.history.index.v1";
const SNAP_PREFIX = "clark.history.snap.v1.";
/** Cap the index so a long-lived install can't grow unbounded. */
const MAX_CONVERSATIONS = 100;

function safeStore(): Storage | null {
  try {
    return typeof localStorage !== "undefined" ? localStorage : null;
  } catch {
    return null;
  }
}

/** All saved conversations, newest first. */
export function loadIndex(): ConversationMeta[] {
  const store = safeStore();
  if (!store) return [];
  try {
    const raw = store.getItem(INDEX_KEY);
    const list = raw ? (JSON.parse(raw) as ConversationMeta[]) : [];
    return list.sort((a, b) => b.updatedAt - a.updatedAt);
  } catch {
    return [];
  }
}

function writeIndex(list: ConversationMeta[]): void {
  const store = safeStore();
  if (!store) return;
  const trimmed = list.sort((a, b) => b.updatedAt - a.updatedAt).slice(0, MAX_CONVERSATIONS);
  // Drop snapshots for any conversations evicted by the cap.
  const kept = new Set(trimmed.map((c) => c.id));
  for (const c of list) if (!kept.has(c.id)) store.removeItem(SNAP_PREFIX + c.id);
  try {
    store.setItem(INDEX_KEY, JSON.stringify(trimmed));
  } catch {
    /* quota — ignore; history is best-effort */
  }
}

/** Insert or update one conversation's metadata. */
export function upsertMeta(meta: ConversationMeta): void {
  const list = loadIndex().filter((c) => c.id !== meta.id);
  list.push(meta);
  writeIndex(list);
}

/** Toggle a conversation's soft-delete (archived) flag. Leaves the snapshot and
 *  index entry intact so it can be restored with its full transcript. */
export function setArchived(id: string, archived: boolean): void {
  const list = loadIndex();
  const found = list.find((c) => c.id === id);
  if (!found || !!found.archived === archived) return;
  found.archived = archived;
  writeIndex(list);
}

/** A persisted transcript is never live: coerce any non-terminal run to a
 *  settled status, and drop a stale permission prompt, so a reopened (or
 *  reloaded) conversation never shows a stuck "Thinking…" or a dead prompt. */
function settleRuns(snapshot: Snapshot): Snapshot {
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
  if (!changed && !snapshot.pending_permission) return snapshot;
  return { ...snapshot, runs, pending_permission: undefined };
}

export function loadSnapshot(id: string): Snapshot | null {
  const store = safeStore();
  if (!store) return null;
  try {
    const raw = store.getItem(SNAP_PREFIX + id);
    return raw ? settleRuns(JSON.parse(raw) as Snapshot) : null;
  } catch {
    return null;
  }
}

export function saveSnapshot(id: string, snapshot: Snapshot): void {
  const store = safeStore();
  if (!store) return;
  try {
    store.setItem(SNAP_PREFIX + id, JSON.stringify(snapshot));
  } catch {
    /* quota — ignore */
  }
}

export function deleteConversation(id: string): void {
  const store = safeStore();
  if (!store) return;
  store.removeItem(SNAP_PREFIX + id);
  writeIndex(loadIndex().filter((c) => c.id !== id));
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
