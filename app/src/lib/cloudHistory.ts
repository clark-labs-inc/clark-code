// Cloud storage for conversation history — Clark's desktop-conversation API.
//
// This is the SOURCE OF TRUTH for chats (no local persistence): the list comes
// from `cloudList`, transcripts from `cloudGet`, writes via `cloudPut` /
// `cloudSetArchived` / `cloudDelete`. Scoped to the signed-in user by the Clark
// JWT, so a second account on the same machine only ever sees its own chats.
// Only available in the desktop app for a signed-in user with a Clark token.

import { invoke } from "@tauri-apps/api/core";
import type { Snapshot } from "../core-bridge/types";
import type { ConversationMeta } from "./history";
import type { AuthSession } from "./auth";
import { prepareSnapshotForUpload } from "./snapshotUpload";
import {
  repositoryFingerprintForRoot,
  repositoryIdentityForRoot,
} from "./repositoryKnowledge";

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
  remoteHost?: string;
  mode?: string;
  titleLocked?: boolean;
  archived?: boolean;
  createdAt: string;
  updatedAt: string;
}

function metaFromSummary(r: CloudSummary): ConversationMeta {
  return {
    id: r.id,
    title: r.title,
    provider: r.provider,
    project: r.project || undefined,
    remoteHost: r.remoteHost || undefined,
    mode: r.mode || undefined,
    titleLocked: r.titleLocked || undefined,
    archived: r.archived || undefined,
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
  status: "running" | "idle",
): Promise<void> {
  const repositoryFingerprint = meta.project
    ? await repositoryFingerprintForRoot(meta.project)
    : null;
  await invoke("desktop_conv_put", {
    endpoint: c.endpoint,
    token: c.token,
    id: meta.id,
    title: meta.title,
    provider: meta.provider,
    project: meta.project ?? null,
    repositoryFingerprint,
    remoteHost: meta.remoteHost ?? null,
    mode: meta.mode ?? null,
    titleLocked: meta.titleLocked ?? false,
    rev,
    snapshot,
    status,
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
  status: "running" | "idle";
}

/** Absolute backstop: even after trimming old tool outputs, skip cloud sync
 *  for a snapshot this large (keep it local only) so one pathological
 *  transcript can't hammer the network. Must stay ≤ the server's desktop
 *  snapshot body limit (`DESKTOP_SNAPSHOT_BODY_LIMIT_BYTES`, currently 10 MiB)
 *  so anything we do send is always accepted rather than silently 413'd. */
const MAX_SNAPSHOT_BYTES = 8 * 1024 * 1024;

const inflight = new Set<string>();
const pending = new Map<string, PendingPush>();
// Fingerprint of the last successfully-PUT payload per conversation. Skipping
// byte-identical re-sends matters twice over: it saves the upload itself, and
// because rev is a timestamp (always "newer"), a no-op PUT would also make
// every mobile/web poller re-download the full snapshot it already has.
const lastSent = new Map<string, string>();

/** Tiny FNV-1a content hash — collision odds are negligible here, and the
 *  worst case is one skipped upload that the next real change re-syncs. */
function fingerprint(job: PendingPush, snapshotJson: string): string {
  const m = job.meta;
  const payload = [
    m.title,
    m.provider,
    m.project ?? "",
    m.project ? repositoryIdentityForRoot(m.project)?.fingerprint ?? "" : "",
    m.remoteHost ?? "",
    m.mode ?? "",
    m.titleLocked ? "1" : "0",
    job.status,
    snapshotJson,
  ].join(" ");
  let h = 0x811c9dc5;
  for (let i = 0; i < payload.length; i++) {
    h ^= payload.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return `${(h >>> 0).toString(16)}:${payload.length}`;
}

/** Queue a conversation snapshot for cloud sync (coalesced + single-flight). */
export function scheduleCloudPut(
  creds: CloudCreds,
  meta: ConversationMeta,
  snapshot: Snapshot,
  status: "running" | "idle" = "idle",
): void {
  // Monotonic rev: pushes now also happen mid-run (throttled), where the
  // timeline length is stable while message text grows — so a length-based
  // rev would make the server drop streamed updates as stale. A millisecond
  // timestamp is strictly increasing across pushes (the 2s throttle + the
  // single-flight queue guarantee spacing), survives restarts, and nothing
  // reads the rev back as a length — it is purely an ordering token.
  const rev = Date.now();
  pending.set(meta.id, { creds, meta, snapshot, rev, status });
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
    // Trim old tool outputs if the snapshot is oversized (no-op for normal
    // conversations), then fingerprint and size-check what we'll actually send.
    const prepared = prepareSnapshotForUpload(job.snapshot);
    const mark = fingerprint(job, prepared.json);
    if (prepared.json.length <= MAX_SNAPSHOT_BYTES && lastSent.get(id) !== mark) {
      await cloudPut(job.creds, job.meta, prepared.snapshot, job.rev, job.status);
      lastSent.set(id, mark);
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
  // A re-created conversation with the same id must upload fresh.
  lastSent.delete(id);
}

/** Toggle a conversation's archived flag in the cloud (independent of snapshot
 *  sync, so archiving never re-uploads the transcript or gets clobbered by it). */
export async function cloudSetArchived(c: CloudCreds, id: string, archived: boolean): Promise<void> {
  await invoke("desktop_conv_set_archived", {
    endpoint: c.endpoint,
    token: c.token,
    id,
    archived,
  });
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
