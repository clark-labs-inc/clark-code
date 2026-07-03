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

const INDEX_KEY_BASE = "clark.history.index.v1";
const SNAP_PREFIX_BASE = "clark.history.snap.v1.";
/** Cap the index so a long-lived install can't grow unbounded. */
const MAX_CONVERSATIONS = 100;

// History is cached PER SIGNED-IN ACCOUNT so a second account on the same
// machine never sees the first account's chats (the WebView shares one
// localStorage origin across accounts). `scope` is set from the account email
// on init and on every auth change via `setHistoryScope`.
let scope = "anon";

/** Point history storage at a specific account (its email). Call on init and
 *  whenever the signed-in account changes. Unscoped keys written before this
 *  change are migrated into the first account that signs in, so upgrading
 *  users keep their existing chats. */
export function setHistoryScope(accountKey: string | null | undefined): void {
  const next = accountKey && accountKey.trim() ? accountKey.trim().toLowerCase() : "anon";
  if (next === scope) return;
  scope = next;
  migrateLegacyGlobal();
}

function indexKey(): string {
  return `${INDEX_KEY_BASE}.${scope}`;
}
function snapKey(id: string): string {
  return `${SNAP_PREFIX_BASE}${scope}.${id}`;
}

function safeStore(): Storage | null {
  try {
    return typeof localStorage !== "undefined" ? localStorage : null;
  } catch {
    return null;
  }
}

/** One-time move of history saved under the old global (unscoped) keys into the
 *  current account scope. Runs when a real scope is first selected and no
 *  scoped index exists yet; leaves the legacy keys in place on any failure. */
function migrateLegacyGlobal(): void {
  const store = safeStore();
  if (!store || scope === "anon") return;
  if (store.getItem(indexKey())) return; // already have data for this account
  const legacy = store.getItem(INDEX_KEY_BASE);
  if (!legacy) return;
  try {
    const list = JSON.parse(legacy) as ConversationMeta[];
    store.setItem(indexKey(), legacy);
    for (const c of list) {
      const snap = store.getItem(SNAP_PREFIX_BASE + c.id);
      if (snap != null) store.setItem(snapKey(c.id), snap);
    }
    store.removeItem(INDEX_KEY_BASE);
    for (const c of list) store.removeItem(SNAP_PREFIX_BASE + c.id);
  } catch {
    /* best-effort — leave legacy keys untouched */
  }
}

/** All saved conversations, newest first. */
export function loadIndex(): ConversationMeta[] {
  const store = safeStore();
  if (!store) return [];
  try {
    const raw = store.getItem(indexKey());
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
  for (const c of list) if (!kept.has(c.id)) store.removeItem(snapKey(c.id));
  try {
    store.setItem(indexKey(), JSON.stringify(trimmed));
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
    const raw = store.getItem(snapKey(id));
    return raw ? settleRuns(JSON.parse(raw) as Snapshot) : null;
  } catch {
    return null;
  }
}

export function saveSnapshot(id: string, snapshot: Snapshot): void {
  const store = safeStore();
  if (!store) return;
  try {
    store.setItem(snapKey(id), JSON.stringify(snapshot));
  } catch {
    /* quota — ignore */
  }
}

export function deleteConversation(id: string): void {
  const store = safeStore();
  if (!store) return;
  store.removeItem(snapKey(id));
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
