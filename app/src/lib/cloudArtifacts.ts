// Mandatory cloud synchronization for Clark Code generated Markdown.
//
// The live snapshot keeps its absolute local URI for instant preview. Before
// cloud persistence, this module replaces that URI with either a durable Clark
// API URI or a workspace-relative retry intent. Upload leases stay inside the
// native host and retries continue until sign-out/account reset or deletion.

import { invoke } from "@tauri-apps/api/core";
import type { Artifact, Snapshot } from "../core-bridge/types";

export interface ArtifactCloudCreds {
  endpoint: string;
  token: string;
}

interface UploadedArtifact {
  artifact_id: string;
  logical_id: string;
  filename: string;
  content_type: string;
  size_bytes: number;
  sha256: string;
  state: string;
  uri: string;
}

interface UploadJob {
  conversationId: string;
  logicalId: string;
  sourceUri: string;
  key: string;
  onReady: () => void;
  onWarning?: (message: string) => void;
  attempts: number;
  epoch: number;
}

const readyUris = new Map<string, string>();
const nonRetryable = new Set<string>();
const jobs = new Map<string, UploadJob>();
const inflight = new Set<string>();
const retryTimers = new Map<string, ReturnType<typeof setTimeout>>();
let configuredCreds: ArtifactCloudCreds | null = null;
let syncEpoch = 0;

const RETRY_INITIAL_MS = 1_000;
const RETRY_MAX_MS = 30_000;

function isMarkdown(artifact: Artifact): boolean {
  return artifact.mime_type?.toLowerCase().startsWith("text/markdown") === true
    || /\.(?:md|markdown)(?:[?#]|$)/i.test(artifact.uri ?? artifact.title);
}

export function isCloudArtifactUri(uri?: string): boolean {
  return /^\/api\/desktop\/conversations\/[^/]+\/artifacts\/[^/?#]+$/.test(uri ?? "");
}

export function isWorkspaceArtifactUri(uri?: string): boolean {
  return uri?.startsWith("clark-workspace://") === true;
}

function localFilesystemUri(uri?: string): boolean {
  if (!uri) return false;
  return uri.startsWith("file://")
    || uri.startsWith("/")
    || /^[a-z]:[\\/]/i.test(uri)
    || uri.startsWith("\\\\");
}

function decodedLocalPath(uri: string): string {
  if (!uri.startsWith("file://")) return uri;
  try {
    return decodeURIComponent(uri.slice("file://".length)).replace(/^localhost/, "");
  } catch {
    return uri.slice("file://".length);
  }
}

function opaqueHash(value: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}

function safeFileName(title: string): string {
  const filename = title.replace(/[/\\\u0000-\u001f\u007f]+/g, "_").trim();
  return filename || "document.md";
}

function workspaceRelativePath(uri: string, conversationId: string): string | null {
  const normalized = decodedLocalPath(uri).replace(/\\/g, "/");
  const marker = "/.clark/workspace/";
  const at = normalized.lastIndexOf(marker);
  if (at < 0) return null;
  const tail = normalized.slice(at + marker.length);
  const slash = tail.indexOf("/");
  if (slash <= 0 || tail.slice(0, slash) !== conversationId) return null;
  const relative = tail.slice(slash + 1);
  if (!relative || relative.split("/").some((part) => !part || part === "." || part === "..")) {
    return null;
  }
  return relative;
}

function pendingWorkspaceUri(
  artifact: Artifact,
  conversationId: string,
): { uri: string; logicalId: string } {
  const relative = isWorkspaceArtifactUri(artifact.uri)
    ? null
    : workspaceRelativePath(artifact.uri ?? "", conversationId);
  if (isWorkspaceArtifactUri(artifact.uri)) {
    const encodedSession = encodeURIComponent(conversationId);
    const prefix = `clark-workspace://${encodedSession}/`;
    if (artifact.uri!.startsWith(prefix)) {
      return { uri: artifact.uri!, logicalId: safeArtifactId(artifact, conversationId) };
    }
  }
  const path = relative ?? safeFileName(artifact.title);
  const encodedPath = path.split("/").map(encodeURIComponent).join("/");
  return {
    uri: `clark-workspace://${encodeURIComponent(conversationId)}/${encodedPath}`,
    logicalId: relative ? `doc:${relative}` : `doc:${safeFileName(artifact.title)}:${opaqueHash(artifact.id)}`,
  };
}

function safeArtifactId(artifact: Artifact, conversationId: string): string {
  if (artifact.id.startsWith("doc:") && !localFilesystemUri(artifact.id.slice(4))) {
    return artifact.id;
  }
  const relative = artifact.uri && workspaceRelativePath(artifact.uri, conversationId);
  return relative
    ? `doc:${relative}`
    : `artifact:${safeFileName(artifact.title)}:${opaqueHash(artifact.id)}`;
}

function uploadJobKey(
  conversationId: string,
  artifact: Artifact,
  sourceUri: string,
): string {
  return [
    conversationId,
    safeArtifactId(artifact, conversationId),
    artifact.tool_call ?? "",
    sourceUri,
  ].join("\u0000");
}

/**
 * Pure cloud projection. It cannot leak an absolute host path in either
 * `artifact.id` or `artifact.uri`; pending Markdown remains retryable after an
 * app restart through `clark-workspace://`.
 */
export function snapshotForArtifactCloud(
  conversationId: string,
  snapshot: Snapshot,
): Snapshot {
  const idReplacements = new Map<string, string>();
  let changed = false;
  const artifacts = snapshot.artifacts.map((artifact) => {
    const uri = artifact.uri;
    if (isCloudArtifactUri(uri)) return artifact;

    const safeId = safeArtifactId(artifact, conversationId);
    idReplacements.set(artifact.id, safeId);
    if (isMarkdown(artifact) && (localFilesystemUri(uri) || isWorkspaceArtifactUri(uri))) {
      const sourceUri = uri!;
      const ready = readyUris.get(uploadJobKey(conversationId, artifact, sourceUri));
      const pending = pendingWorkspaceUri(artifact, conversationId);
      changed ||= safeId !== artifact.id || (ready ?? pending.uri) !== uri;
      return {
        ...artifact,
        id: safeId,
        uri: ready ?? pending.uri,
      };
    }
    if (localFilesystemUri(uri)) {
      changed = true;
      return { ...artifact, id: safeId, uri: undefined };
    }
    changed ||= safeId !== artifact.id;
    return safeId === artifact.id ? artifact : { ...artifact, id: safeId };
  });
  if (!changed) return snapshot;
  const timeline = snapshot.timeline.map((item) => {
    if (item.item !== "artifact") return item;
    const id = idReplacements.get(item.id);
    return id && id !== item.id ? { ...item, id } : item;
  });
  return { ...snapshot, artifacts, timeline };
}

function clearTimer(key: string): void {
  const timer = retryTimers.get(key);
  if (timer !== undefined) clearTimeout(timer);
  retryTimers.delete(key);
}

function retry(job: UploadJob): void {
  if (job.epoch !== syncEpoch || retryTimers.has(job.key) || !jobs.has(job.key)) return;
  const delay = Math.min(RETRY_INITIAL_MS * (2 ** job.attempts), RETRY_MAX_MS);
  job.attempts += 1;
  retryTimers.set(job.key, setTimeout(() => {
    retryTimers.delete(job.key);
    void drain(job.key);
  }, delay));
}

async function drain(key: string): Promise<void> {
  const job = jobs.get(key);
  const creds = configuredCreds;
  if (!job || !creds || inflight.has(key) || job.epoch !== syncEpoch) return;
  inflight.add(key);
  clearTimer(key);
  try {
    const artifact = await invoke<UploadedArtifact>("desktop_artifact_upload", {
      endpoint: creds.endpoint,
      token: creds.token,
      desktopId: job.conversationId,
      logicalId: job.logicalId,
      sourceUri: job.sourceUri,
    });
    if (
      job.epoch !== syncEpoch
      || jobs.get(key) !== job
      || artifact.state !== "uploaded"
      || !isCloudArtifactUri(artifact.uri)
    ) {
      throw new Error("Clark did not confirm the uploaded artifact");
    }
    readyUris.set(key, artifact.uri);
    jobs.delete(key);
    job.onReady();
  } catch (error) {
    const message = String(error);
    const sourceMissing = /artifact is unavailable|workspace is unavailable|no Clark workspace/i.test(
      message,
    );
    const permanentlyInvalid = /exceeds the 8 MB|not Markdown|invalid artifact|quota exceeded|400 Bad Request|403 Forbidden|413 Payload Too Large/i.test(
      message,
    );
    if (sourceMissing || permanentlyInvalid) {
      jobs.delete(key);
      nonRetryable.add(key);
      if (permanentlyInvalid) {
        job.onWarning?.(
          "A generated document could not be saved to Clark cloud. It remains available locally.",
        );
      }
    } else {
      retry(job);
    }
  } finally {
    inflight.delete(key);
  }
}

/** Discover generated Markdown and guarantee it has an active upload/retry job. */
export function scheduleArtifactCloudSync(
  creds: ArtifactCloudCreds,
  conversationId: string,
  snapshot: Snapshot,
  onReady: () => void,
  onWarning?: (message: string) => void,
): void {
  if (
    configuredCreds?.endpoint !== creds.endpoint
    || configuredCreds?.token !== creds.token
  ) return;
  for (const artifact of snapshot.artifacts) {
    const sourceUri = artifact.uri;
    if (
      !sourceUri
      || !isMarkdown(artifact)
      || isCloudArtifactUri(sourceUri)
      || (!localFilesystemUri(sourceUri) && !isWorkspaceArtifactUri(sourceUri))
    ) continue;
    const key = uploadJobKey(conversationId, artifact, sourceUri);
    if (readyUris.has(key) || nonRetryable.has(key)) continue;
    const pending = pendingWorkspaceUri(artifact, conversationId);
    const existing = jobs.get(key);
    if (existing) {
      existing.onReady = onReady;
      existing.onWarning = onWarning;
      continue;
    }
    const job: UploadJob = {
      conversationId,
      logicalId: pending.logicalId,
      sourceUri,
      key,
      onReady,
      onWarning,
      attempts: 0,
      epoch: syncEpoch,
    };
    jobs.set(key, job);
    void drain(key);
  }
}

export function configureArtifactCloudCredentials(creds: ArtifactCloudCreds | null): void {
  configuredCreds = creds ? { ...creds } : null;
  if (configuredCreds) {
    for (const key of jobs.keys()) void drain(key);
  }
}

export function resetArtifactCloudSync(): void {
  syncEpoch += 1;
  configuredCreds = null;
  for (const timer of retryTimers.values()) clearTimeout(timer);
  retryTimers.clear();
  jobs.clear();
  readyUris.clear();
  nonRetryable.clear();
}

export function forgetArtifactCloudConversation(conversationId: string): void {
  for (const [key, job] of jobs) {
    if (job.conversationId !== conversationId) continue;
    clearTimer(key);
    jobs.delete(key);
  }
  for (const key of readyUris.keys()) {
    if (key.startsWith(`${conversationId}\u0000`)) readyUris.delete(key);
  }
  for (const key of nonRetryable) {
    if (key.startsWith(`${conversationId}\u0000`)) nonRetryable.delete(key);
  }
}

export async function flushArtifactCloudSync(timeoutMs: number): Promise<boolean> {
  for (const key of jobs.keys()) void drain(key);
  const deadline = Date.now() + timeoutMs;
  while (jobs.size > 0 || inflight.size > 0) {
    if (Date.now() >= deadline) return false;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  return true;
}
