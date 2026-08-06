// Cloud storage for conversation history — Clark's desktop-conversation API.
//
// The cloud database is the SOURCE OF TRUTH for chats: the list comes from
// `cloudList`, transcripts from `cloudGet`, and mutations use `cloudPut` /
// `cloudSetArchived` / `cloudDelete`. The Rust bridge maintains an
// account-scoped SQLite outbox and acknowledged snapshot cache for atomic local
// writes, offline delivery, and restart recovery; it reconciles against cloud
// revisions and cannot overwrite a newer cloud snapshot. The native host binds
// the validated Clark JWT subject to the local records, so the WebView cannot
// select another account's partition. Cloud sync is available only in the
// desktop app for a native-retained signed-in account.

import { invoke } from "@tauri-apps/api/core";
import { normalizeSnapshot, type Snapshot } from "../core-bridge/types";
import { migratePlanningSnapshot, type ConversationMeta } from "./history";
import type { AuthSession } from "./auth";
import { codeKeyAccountBinding } from "./account";
import {
  metaFromDetail,
  metaFromSummary,
  type CloudDetail,
  type CloudSummary,
} from "./cloudHistoryTypes";
import { prepareSnapshotForUpload } from "./snapshotUpload";
import {
  repositoryFingerprintForRoot,
  repositoryIdentityForRoot,
} from "./repositoryKnowledge";
import {
  configureArtifactCloudCredentials,
  flushArtifactCloudSync,
  forgetArtifactCloudConversation,
  resetArtifactCloudSync,
  scheduleArtifactCloudSync,
  snapshotForArtifactCloud,
} from "./cloudArtifacts";

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export interface CloudCreds {
  /** Stable local partition key for account-bound client caches. */
  accountScope: string;
}

/** Non-secret cloud descriptor, or null outside a signed-in desktop host. */
export function cloudCreds(auth: AuthSession | null): CloudCreds | null {
  if (!isTauri()) return null;
  const accountScope = codeKeyAccountBinding(auth);
  if (!accountScope) return null;
  return { accountScope };
}

/** List the user's cloud conversations (metadata only). */
export async function cloudList(c: CloudCreds): Promise<ConversationMeta[]> {
  const epoch = cloudHistoryEpoch;
  const rows = await invoke<CloudSummary[] | null>("desktop_conv_list");
  if (epoch !== cloudHistoryEpoch || !sameCloudOwner(configuredCreds, c)) return [];
  const summaries = (rows ?? []).map(metaFromSummary);
  for (const summary of summaries) serverRevisions.set(summary.id, summary.rev ?? 0);
  return summaries;
}

/** Fetch one cloud conversation's snapshot, or null if absent. */
export async function cloudGet(c: CloudCreds, id: string): Promise<Snapshot | null> {
  const epoch = cloudHistoryEpoch;
  const detail = await invoke<CloudDetail | null>("desktop_conv_get", {
    id,
  });
  if (epoch !== cloudHistoryEpoch || !sameCloudOwner(configuredCreds, c)) return null;
  if (typeof detail?.rev === "number") {
    serverRevisions.set(id, detail.rev);
    conflicted.delete(id);
  }
  // Durable cloud history can outlive the snapshot schema that produced it.
  // Normalize legacy planning rows before the snapshot crosses the native
  // `session_configure_cloud` boundary, whose Rust enum intentionally accepts
  // only the current typed timeline variants.
  const normalizedWire = detail?.snapshot ? normalizeSnapshot(detail.snapshot) : null;
  const providerOutputQuarantined = Boolean(
    detail?.snapshot
      && normalizedWire
      && (normalizedWire.timeline.length < detail.snapshot.timeline.length
        || Object.keys(normalizedWire.tool_calls).length
          < Object.keys(detail.snapshot.tool_calls).length
        || (detail.snapshot.model_context_checkpoint
          && !normalizedWire.model_context_checkpoint)),
  );
  const snapshot = normalizedWire ? migratePlanningSnapshot(normalizedWire) : null;
  if (detail && (detail.snapshotRecoveryRequired || providerOutputQuarantined) && snapshot) {
    const meta = metaFromDetail(detail);
    if (meta) {
      // Native recovery already wrote the terminal events to its durable outbox.
      // Publish the exact recovered projection once that prefix is acknowledged;
      // otherwise mobile keeps reading the pre-restart running snapshot forever.
      // The same write permanently removes quarantined provider residue instead
      // of merely hiding it in this renderer.
      scheduleCloudPut(c, meta, snapshot, "idle");
    }
  }
  return snapshot;
}

/** Upsert a conversation snapshot with optimistic concurrency. The stable
 * mutation ID makes a retry idempotent; a mismatched base revision conflicts. */
async function cloudPut(
  c: CloudCreds,
  meta: ConversationMeta,
  snapshot: Snapshot,
  rev: number,
  status: "running" | "idle",
  baseRev: number,
  mutationId: string,
  shouldSend: () => boolean = () => true,
): Promise<number> {
  const repositoryFingerprint = meta.project
    ? await repositoryFingerprintForRoot(meta.project, c.accountScope)
    : null;
  if (!shouldSend()) throw new CloudWriteCancelled();
  const summary = await invoke<{ rev: number }>("desktop_conv_put", {
    id: meta.id,
    title: meta.title,
    provider: meta.provider,
    project: meta.project ?? null,
    repositoryFingerprint,
    remoteHost: meta.remoteHost ?? null,
    mode: meta.mode ?? null,
    titleLocked: meta.titleLocked ?? false,
    specialistContext: meta.specialist ?? null,
    rev,
    snapshot,
    status,
    baseRev,
    mutationId,
  });
  return summary.rev;
}

// --- Single-flight, coalescing write pipeline -----------------------------
//
// Per conversation we keep at most one PUT in flight and at most one queued
// "latest" snapshot. Rapid saves collapse to the newest; transient failures
// retry with bounded backoff (idempotent on the server, so retries are safe and
// never duplicate). This keeps writes fast and non-blocking and gives at-most-
// one in-flight delivery per conversation.

interface PendingPush {
  creds: CloudCreds;
  meta: ConversationMeta;
  snapshot: Snapshot;
  rev: number;
  status: "running" | "idle";
  mutationId: string;
  epoch: number;
}

/** Absolute backstop: skip cloud sync for a snapshot this large so one pathological
 *  transcript can't hammer the network. Must stay ≤ the server's desktop
 *  snapshot body limit (`DESKTOP_SNAPSHOT_BODY_LIMIT_BYTES`, currently 10 MiB)
 *  so anything we do send is always accepted rather than silently 413'd. */
export const MAX_SNAPSHOT_BYTES = 8 * 1024 * 1024;

const inflight = new Set<string>();
const pending = new Map<string, PendingPush>();
// Fingerprint of the last successfully-PUT payload per conversation. Skipping
// byte-identical re-sends matters twice over: it saves the upload itself, and
// because rev is a timestamp (always "newer"), a no-op PUT would also make
// every mobile/web poller re-download the full snapshot it already has.
const lastSent = new Map<string, string>();
const serverRevisions = new Map<string, number>();
const conflicted = new Set<string>();
const retryTimers = new Map<string, ReturnType<typeof setTimeout>>();
const retryAttempts = new Map<string, number>();
const deleting = new Set<string>();
const deleteGenerations = new Map<string, number>();
let cloudHistoryEpoch = 0;
let nextDeleteGeneration = 0;
let configuredCreds: CloudCreds | null = null;
let conflictHandler: ((conversationId: string) => void) | null = null;
let warningHandler: ((message: string) => void) | null = null;

const RETRY_INITIAL_DELAY_MS = 1_000;
const RETRY_MAX_DELAY_MS = 30_000;

function sameCreds(left: CloudCreds | null, right: CloudCreds | null): boolean {
  return left?.accountScope === right?.accountScope;
}

function sameCloudOwner(left: CloudCreds | null, right: CloudCreds | null): boolean {
  return Boolean(left && right && left.accountScope === right.accountScope);
}

/** Bind queued snapshot writes to the currently authenticated Desktop account.
 * Refreshes retain the queue; sign-out and account changes must call
 * `resetCloudHistory` first so no stale job can inherit another account's JWT. */
export function configureCloudHistoryCredentials(creds: CloudCreds | null): void {
  configuredCreds = creds ? { ...creds } : null;
  configureArtifactCloudCredentials(configuredCreds);
}

/** Stop all queued retry work at an account boundary. In-flight requests cannot
 * be cancelled after crossing the native boundary, but their completions are
 * fenced by this epoch and therefore cannot requeue or repopulate local state. */
export function resetCloudHistory(): void {
  cloudHistoryEpoch += 1;
  configuredCreds = null;
  for (const timer of retryTimers.values()) clearTimeout(timer);
  retryTimers.clear();
  retryAttempts.clear();
  pending.clear();
  lastSent.clear();
  serverRevisions.clear();
  conflicted.clear();
  deleting.clear();
  deleteGenerations.clear();
  resetArtifactCloudSync();
}

function jobIsCurrent(id: string, job: PendingPush): boolean {
  return job.epoch === cloudHistoryEpoch && !deleting.has(id);
}

class CloudWriteCancelled extends Error {}

function clearRetry(id: string): void {
  const timer = retryTimers.get(id);
  if (timer !== undefined) clearTimeout(timer);
  retryTimers.delete(id);
}

function retryPendingPush(id: string): void {
  const job = pending.get(id);
  if (
    retryTimers.has(id)
    || conflicted.has(id)
    || !job
    || !jobIsCurrent(id, job)
    || !configuredCreds
  ) {
    return;
  }
  const attempts = retryAttempts.get(id) ?? 0;
  retryAttempts.set(id, attempts + 1);
  const delay = Math.min(RETRY_INITIAL_DELAY_MS * (2 ** attempts), RETRY_MAX_DELAY_MS);
  const timer = setTimeout(() => {
    retryTimers.delete(id);
    void drainPush(id);
  }, delay);
  retryTimers.set(id, timer);
}

/** The store installs this once so a concurrent-device write conflict is
 * visible and the stale snapshot is not retried forever. */
export function onCloudHistoryConflict(handler: (conversationId: string) => void): () => void {
  conflictHandler = handler;
  return () => {
    if (conflictHandler === handler) conflictHandler = null;
  };
}

/** The store installs this once so permanent client-side sync failures are as
 * visible as native outbox delivery warnings. */
export function onCloudHistoryWarning(handler: (message: string) => void): () => void {
  warningHandler = handler;
  return () => {
    if (warningHandler === handler) warningHandler = null;
  };
}

function mutationId(): string {
  return globalThis.crypto?.randomUUID?.()
    ?? `desktop-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

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
  // Never let a stale render callback resurrect work after sign-out or switch
  // an account's queued transcript onto a different native account generation.
  if (!sameCreds(configuredCreds, creds) || deleting.has(meta.id)) return;
  // A stale branch stays read-only until cloudGet reloads its exact base.
  if (conflicted.has(meta.id)) return;
  // Monotonic rev: pushes now also happen mid-run (throttled), where the
  // timeline length is stable while message text grows — so a length-based
  // rev would make the server drop streamed updates as stale. A millisecond
  // timestamp is strictly increasing across pushes (the 2s throttle + the
  // single-flight queue guarantee spacing), survives restarts, and nothing
  // reads the rev back as a length — it is purely an ordering token.
  const rev = Date.now();
  scheduleArtifactCloudSync(creds, meta.id, snapshot, () => {
    const current = configuredCreds;
    if (current) scheduleCloudPut(current, meta, snapshot, status);
  }, warningHandler ?? undefined);
  const cloudSafeSnapshot = snapshotForArtifactCloud(meta.id, snapshot);
  clearRetry(meta.id);
  pending.set(meta.id, {
    creds,
    meta,
    snapshot: cloudSafeSnapshot,
    rev,
    status,
    mutationId: mutationId(),
    epoch: cloudHistoryEpoch,
  });
  void drainPush(meta.id);
}

/** Wait for every currently queued/in-flight snapshot write to settle. The
 *  updater calls this only after runs and follow-ups drain, so no new streaming
 *  snapshots should appear. Returns false on a failed or timed-out delivery;
 *  callers can keep the app running and retry instead of losing the final tail. */
export async function flushCloudPuts(timeoutMs = 5000): Promise<boolean> {
  const startedAt = Date.now();
  if (!(await flushArtifactCloudSync(timeoutMs))) return false;
  for (const id of [...pending.keys()]) void drainPush(id);
  const deadline = startedAt + timeoutMs;
  while (inflight.size > 0) {
    if (Date.now() >= deadline) return false;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  return pending.size === 0;
}

async function drainPush(id: string): Promise<void> {
  if (inflight.has(id) || deleting.has(id)) return; // already sending or deleting this conversation
  const job = pending.get(id);
  if (!job || !jobIsCurrent(id, job)) {
    if (pending.get(id) === job) pending.delete(id);
    return;
  }
  clearRetry(id);
  pending.delete(id);
  inflight.add(id);
  let ok = false;
  try {
    // Serialize exactly, then fingerprint and size-check what we'll send.
    const prepared = prepareSnapshotForUpload(job.snapshot);
    const mark = fingerprint(job, prepared.json);
    if (prepared.bytes > MAX_SNAPSHOT_BYTES) {
      // Keep the job queued. Marking this successful would let the native
      // trajectory outbox advance its checkpoint while the cloud snapshot
      // stayed stale, permanently losing cross-device reconstruction.
      if (jobIsCurrent(id, job) && !pending.has(id)) pending.set(id, job);
      warningHandler?.(
        "This conversation is too large to sync safely. Its latest history remains on this device; start a new conversation to restore cloud sync.",
      );
      return;
    }
    if (lastSent.get(id) !== mark) {
      const creds = configuredCreds;
      if (!creds) throw new CloudWriteCancelled();
      const storedRev = await cloudPut(
        creds,
        job.meta,
        prepared.snapshot,
        job.rev,
        job.status,
        serverRevisions.get(id) ?? job.meta.rev ?? 0,
        job.mutationId,
        () => jobIsCurrent(id, job) && sameCreds(configuredCreds, creds),
      );
      if (jobIsCurrent(id, job)) {
        serverRevisions.set(id, storedRev);
        lastSent.set(id, mark);
      }
    }
    if (jobIsCurrent(id, job)) {
      ok = true;
      retryAttempts.delete(id);
    }
  } catch (error) {
    if (!jobIsCurrent(id, job)) {
      // A sign-out/account switch may have queued fresh work while this older
      // request was in flight. `finally` below wakes that newer generation.
    } else if (error instanceof CloudWriteCancelled) {
      const creds = configuredCreds;
      if (creds && !pending.has(id)) pending.set(id, { ...job, creds });
      ok = true;
    } else if (String(error).includes("cloud_conflict")) {
      conflicted.add(id);
      pending.delete(id);
      clearRetry(id);
      retryAttempts.delete(id);
      conflictHandler?.(id);
      ok = true;
    } else if (String(error).includes("cloud_deleted:")) {
      pending.delete(id);
      clearRetry(id);
      retryAttempts.delete(id);
      warningHandler?.(
        "This conversation was deleted on another device, so Clark Code stopped syncing it here.",
      );
      ok = true;
    } else {
      // Transient (offline / backend not deployed). Requeue unless a newer push
      // already superseded it, then retry without needing another user action.
      const creds = configuredCreds;
      if (creds && !pending.has(id)) pending.set(id, { ...job, creds });
      retryPendingPush(id);
    }
  } finally {
    inflight.delete(id);
  }
  // Chain a newer snapshot after success. An account reset can replace a job
  // while its old request is still in flight, so it also needs one fresh drain.
  const next = pending.get(id);
  if (
    next
    && jobIsCurrent(id, next)
    && (ok || job.epoch !== cloudHistoryEpoch)
  ) {
    void drainPush(id);
  }
}

async function waitForInflightPush(id: string): Promise<void> {
  while (inflight.has(id)) {
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}

function forgetConversationPushState(id: string): void {
  pending.delete(id);
  clearRetry(id);
  retryAttempts.delete(id);
  lastSent.delete(id);
  serverRevisions.delete(id);
  conflicted.delete(id);
}

/** Delete a conversation from the cloud. Tombstone local write scheduling first,
 * then wait for any already-dispatched PUT before issuing DELETE so a retry can
 * never recreate the conversation behind the user's back. */
export async function cloudDelete(c: CloudCreds, id: string): Promise<void> {
  if (!sameCreds(configuredCreds, c)) return;
  const epoch = cloudHistoryEpoch;
  const generation = ++nextDeleteGeneration;
  deleting.add(id);
  forgetArtifactCloudConversation(id);
  deleteGenerations.set(id, generation);
  forgetConversationPushState(id);
  try {
    await waitForInflightPush(id);
    const creds = configuredCreds;
    if (epoch !== cloudHistoryEpoch || !creds) return;
    await invoke("desktop_conv_delete", {
      id,
    });
  } finally {
    if (deleteGenerations.get(id) === generation) {
      deleteGenerations.delete(id);
      deleting.delete(id);
    }
  }
}

export { cloudSetArchived, cloudShare, cloudUnshare } from "./cloudHistoryMutations";
