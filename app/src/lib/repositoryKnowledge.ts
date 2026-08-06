import { invoke } from "@tauri-apps/api/core";
import type { CloudCreds } from "./cloudHistory";
import {
  organizationForRepository,
  uploadOrganizationRepositoryBatch,
} from "./organizationKnowledge";
import { accountScopedKey } from "./accountProjectStorage";

const ENABLED_KEY = "clark-desktop:project-knowledge-enabled";
const CURSOR_PREFIX = "clark-desktop:project-knowledge-cursor:";
const HISTORY_BATCH_SIZE = 200;
const MAX_BATCHES_PER_PASS = 4;
const MAX_REPOSITORIES_PER_PASS = 4;

export interface RepositoryRemote {
  name: string;
  url: string;
  canonical: string;
}

export interface RepositoryIdentity {
  fingerprint: string;
  vcs: "git";
  root: string;
  head_oid?: string | null;
  current_branch?: string | null;
  default_branch?: string | null;
  canonical_remote?: string | null;
  remotes: RepositoryRemote[];
  commit_count: number;
  shallow: boolean;
  dirty: boolean;
  refs_fingerprint: string;
}

export interface GitCommitEvidence {
  oid: string;
  parent_oids: string[];
  author_name: string;
  author_email: string;
  authored_at: string;
  committed_at: string;
  subject: string;
  body: string;
}

interface GitHistoryBatch {
  repository: RepositoryIdentity;
  offset: number;
  next_offset: number;
  complete: boolean;
  commits: GitCommitEvidence[];
}

interface SyncResponse {
  next_offset: number;
  complete: boolean;
  reset_required: boolean;
}

interface SyncCursor {
  refsFingerprint: string;
  offset: number;
  complete: boolean;
}

const identities = new Map<string, RepositoryIdentity>();
const discoveredRoots = new Map<string, string[]>();
const discoveryOffsets = new Map<string, number>();

export function projectKnowledgeEnabled(scope?: string | null): boolean {
  try {
    return localStorage.getItem(accountScopedKey(ENABLED_KEY, scope)) === "1";
  } catch {
    return false;
  }
}

export function setProjectKnowledgeEnabled(enabled: boolean, scope?: string | null): void {
  try {
    localStorage.setItem(accountScopedKey(ENABLED_KEY, scope), enabled ? "1" : "0");
    window.dispatchEvent(new CustomEvent("clark:project-knowledge-setting"));
  } catch {
    // Storage can be unavailable in hardened browser previews.
  }
}

export function repositoryIdentityForRoot(root: string): RepositoryIdentity | null {
  return identities.get(root.trim()) ?? null;
}

export function repositoriesUnderRoot(root: string): RepositoryIdentity[] {
  const normalized = root.trim();
  const roots = discoveredRoots.get(normalized) ?? [];
  const repositories = roots
    .map((repositoryRoot) => identities.get(repositoryRoot))
    .filter((repository): repository is RepositoryIdentity => Boolean(repository));
  const direct = repositoryIdentityForRoot(normalized);
  if (direct && !repositories.some((repository) => repository.root === direct.root)) {
    repositories.unshift(direct);
  }
  return repositories;
}

export async function discoverRepositories(
  root: string,
  scope?: string | null,
): Promise<RepositoryIdentity[]> {
  const normalized = root.trim();
  if (!normalized || !projectKnowledgeEnabled(scope)) return [];
  try {
    const repositories = await invoke<RepositoryIdentity[]>("clark_repository_discover", {
      cwd: normalized,
    });
    for (const repository of repositories) identities.set(repository.root, repository);
    discoveredRoots.set(normalized, repositories.map((repository) => repository.root));
    return repositories;
  } catch {
    return repositoriesUnderRoot(normalized);
  }
}

export async function repositoryFingerprintForRoot(
  root: string,
  scope?: string | null,
): Promise<string | null> {
  const normalized = root.trim();
  if (!normalized || !projectKnowledgeEnabled(scope)) return null;
  return (
    repositoryIdentityForRoot(normalized) ??
    (await refreshRepositoryIdentity(normalized, scope))
  )?.fingerprint ?? null;
}

export async function refreshRepositoryIdentity(
  root: string,
  scope?: string | null,
): Promise<RepositoryIdentity | null> {
  const normalized = root.trim();
  if (!normalized || !projectKnowledgeEnabled(scope)) return null;
  try {
    const identity = await invoke<RepositoryIdentity | null>("clark_repository_inspect", {
      cwd: normalized,
    });
    if (identity) identities.set(normalized, identity);
    else identities.delete(normalized);
    return identity;
  } catch {
    return repositoryIdentityForRoot(normalized);
  }
}

export async function syncRepositoryHistory(creds: CloudCreds, root: string): Promise<void> {
  if (!projectKnowledgeEnabled(creds.accountScope)) return;
  const initialIdentity = await refreshRepositoryIdentity(root, creds.accountScope);
  if (!initialIdentity) return;
  let identity: RepositoryIdentity = initialIdentity;
  let cursor = loadCursor(identity, creds.accountScope);

  if (cursor.complete) {
    const heartbeat: GitHistoryBatch = {
      repository: identity,
      offset: cursor.offset,
      next_offset: cursor.offset,
      complete: true,
      commits: [],
    };
    const response = await upload(creds, heartbeat);
    if (response.reset_required) {
      cursor = freshCursor(identity);
      saveCursor(identity.fingerprint, cursor, creds.accountScope);
    }
    return;
  }

  for (let pass = 0; pass < MAX_BATCHES_PER_PASS && !cursor.complete; pass += 1) {
    const batch: GitHistoryBatch | null = await invoke("clark_repository_history", {
      cwd: identity.root,
      offset: cursor.offset,
      limit: HISTORY_BATCH_SIZE,
    });
    if (!batch) return;
    identity = batch.repository;
    identities.set(root.trim(), identity);
    if (cursor.refsFingerprint !== identity.refs_fingerprint) {
      cursor = freshCursor(identity);
      if (batch.offset !== 0) continue;
    }
    const response = await upload(creds, batch);
    if (response.reset_required) {
      cursor = freshCursor(identity);
      saveCursor(identity.fingerprint, cursor, creds.accountScope);
      continue;
    }
    cursor = {
      refsFingerprint: identity.refs_fingerprint,
      offset: response.next_offset,
      complete: response.complete,
    };
    saveCursor(identity.fingerprint, cursor, creds.accountScope);
  }
}

export async function syncRepositoriesUnderRoot(
  creds: CloudCreds,
  root: string,
): Promise<void> {
  const normalized = root.trim();
  const cached = repositoriesUnderRoot(normalized);
  const repositories = cached.length > 0
    ? cached
    : await discoverRepositories(normalized, creds.accountScope);
  if (repositories.length === 0) return;
  const start = (discoveryOffsets.get(normalized) ?? 0) % repositories.length;
  const count = Math.min(MAX_REPOSITORIES_PER_PASS, repositories.length);
  for (let index = 0; index < count; index += 1) {
    const repository = repositories[(start + index) % repositories.length];
    await syncRepositoryHistory(creds, repository.root);
  }
  discoveryOffsets.set(normalized, (start + count) % repositories.length);
}

function upload(creds: CloudCreds, batch: GitHistoryBatch): Promise<SyncResponse> {
  const organizationId = organizationForRepository(
    batch.repository.fingerprint,
    creds.accountScope,
  );
  if (organizationId) {
    return uploadOrganizationRepositoryBatch<SyncResponse>(creds, organizationId, batch);
  }
  return invoke<SyncResponse>("desktop_code_repository_sync", {
    batch,
  });
}

function freshCursor(identity: RepositoryIdentity): SyncCursor {
  return { refsFingerprint: identity.refs_fingerprint, offset: 0, complete: false };
}

function loadCursor(identity: RepositoryIdentity, scope?: string | null): SyncCursor {
  try {
    const raw = localStorage.getItem(
      `${accountScopedKey(CURSOR_PREFIX, scope)}${identity.fingerprint}`,
    );
    if (!raw) return freshCursor(identity);
    const value = JSON.parse(raw) as Partial<SyncCursor>;
    if (
      value.refsFingerprint !== identity.refs_fingerprint ||
      typeof value.offset !== "number" ||
      typeof value.complete !== "boolean"
    ) {
      return freshCursor(identity);
    }
    return {
      refsFingerprint: value.refsFingerprint,
      offset: Math.max(0, Math.floor(value.offset)),
      complete: value.complete,
    };
  } catch {
    return freshCursor(identity);
  }
}

function saveCursor(
  fingerprint: string,
  cursor: SyncCursor,
  scope?: string | null,
): void {
  try {
    localStorage.setItem(
      `${accountScopedKey(CURSOR_PREFIX, scope)}${fingerprint}`,
      JSON.stringify(cursor),
    );
  } catch {
    // A later pass can safely replay because repository commits are idempotent.
  }
}
