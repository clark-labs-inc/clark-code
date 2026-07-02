// Cloud sync for conversation history — Clark's desktop-conversation API.
//
// localStorage (lib/history.ts) stays the synchronous, offline-first cache; this
// layer mirrors it to Clark so a user's coding history is durable and follows
// them across machines. Every call is best-effort: it's only available in the
// desktop app for a signed-in user with a Clark token, and callers swallow
// failures so the app degrades to local-only.

import { invoke } from "@tauri-apps/api/core";
import type { Snapshot } from "../core-bridge/types";
import type { ConversationMeta } from "./history";
import type { AuthSession } from "./auth";

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export interface CloudCreds {
  endpoint: string;
  token: string;
}

/** Cloud creds, or null when sync isn't possible (browser preview or signed out
 *  without a Clark token). */
export function cloudCreds(auth: AuthSession | null): CloudCreds | null {
  if (!isTauri()) return null;
  const endpoint = auth?.clark.endpoint;
  const token = auth?.clark.token;
  if (!endpoint || !token) return null;
  return { endpoint, token };
}

interface CloudSummary {
  id: string;
  title: string;
  provider: string;
  project?: string;
  createdAt: string;
  updatedAt: string;
}

function metaFromSummary(r: CloudSummary): ConversationMeta {
  return {
    id: r.id,
    title: r.title,
    provider: r.provider,
    project: r.project || undefined,
    createdAt: Date.parse(r.createdAt) || Date.now(),
    updatedAt: Date.parse(r.updatedAt) || Date.now(),
  };
}

/** List the user's cloud conversations (metadata only). */
export async function cloudList(c: CloudCreds): Promise<ConversationMeta[]> {
  const rows = await invoke<CloudSummary[] | null>("desktop_conv_list", {
    endpoint: c.endpoint,
    token: c.token,
  });
  return (rows ?? []).map(metaFromSummary);
}

/** Fetch one cloud conversation's snapshot, or null if absent. */
export async function cloudGet(c: CloudCreds, id: string): Promise<Snapshot | null> {
  const detail = await invoke<{ snapshot?: Snapshot } | null>("desktop_conv_get", {
    endpoint: c.endpoint,
    token: c.token,
    id,
  });
  return detail?.snapshot ?? null;
}

/** Upsert a conversation's snapshot in the cloud. `rev` is a monotonic revision
 *  so the server ignores stale/duplicate deliveries (idempotent, at-most-once). */
export async function cloudPut(
  c: CloudCreds,
  meta: ConversationMeta,
  snapshot: Snapshot,
  rev: number,
): Promise<void> {
  await invoke("desktop_conv_put", {
    endpoint: c.endpoint,
    token: c.token,
    id: meta.id,
    title: meta.title,
    provider: meta.provider,
    project: meta.project ?? null,
    rev,
    snapshot,
  });
}

// --- Single-flight, coalescing write pipeline -----------------------------
//
// Per conversation we keep at most one PUT in flight and at most one queued
// "latest" snapshot. Rapid saves collapse to the newest; a failed send is left
// queued for the next turn to retry (idempotent on the server, so retries are
// safe and never duplicate). This keeps writes fast and non-blocking and gives
// at-most-one in-flight delivery per conversation.

interface PendingPush {
  creds: CloudCreds;
  meta: ConversationMeta;
  snapshot: Snapshot;
  rev: number;
}

/** Skip cloud sync for absurdly large snapshots (keep them local only) so one
 *  pathological transcript can't hammer the network. */
const MAX_SNAPSHOT_BYTES = 8 * 1024 * 1024;

const inflight = new Set<string>();
const pending = new Map<string, PendingPush>();

/** Queue a conversation snapshot for cloud sync (coalesced + single-flight). */
export function scheduleCloudPut(
  creds: CloudCreds,
  meta: ConversationMeta,
  snapshot: Snapshot,
): void {
  // Monotonic rev: we only push on turn completion, and each turn appends to the
  // timeline, so its length strictly increases per push.
  const rev = snapshot.timeline.length;
  pending.set(meta.id, { creds, meta, snapshot, rev });
  void drainPush(meta.id);
}

async function drainPush(id: string): Promise<void> {
  if (inflight.has(id)) return; // already sending this conversation
  const job = pending.get(id);
  if (!job) return;
  pending.delete(id);
  inflight.add(id);
  let ok = false;
  try {
    if (JSON.stringify(job.snapshot).length <= MAX_SNAPSHOT_BYTES) {
      await cloudPut(job.creds, job.meta, job.snapshot, job.rev);
    }
    ok = true;
  } catch {
    // Transient (offline / backend not deployed). Requeue unless a newer push
    // already superseded it; the next turn retries. No tight retry loop.
    if (!pending.has(id)) pending.set(id, job);
  } finally {
    inflight.delete(id);
  }
  // Only chain-drain on success — a newer snapshot may be waiting.
  if (ok && pending.has(id)) void drainPush(id);
}

/** Delete a conversation from the cloud. */
export async function cloudDelete(c: CloudCreds, id: string): Promise<void> {
  await invoke("desktop_conv_delete", { endpoint: c.endpoint, token: c.token, id });
}

/** Create (or fetch) the public share link for a synced conversation. */
export async function cloudShare(c: CloudCreds, id: string): Promise<string> {
  const out = await invoke<{ share_url?: string }>("desktop_conv_share", {
    endpoint: c.endpoint,
    token: c.token,
    id,
  });
  if (!out.share_url) throw new Error("Clark did not return a share link.");
  return out.share_url;
}

/** Stop sharing a conversation (revokes the public link). */
export async function cloudUnshare(c: CloudCreds, id: string): Promise<void> {
  await invoke("desktop_conv_unshare", { endpoint: c.endpoint, token: c.token, id });
}
