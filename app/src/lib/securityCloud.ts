import { invoke } from "@tauri-apps/api/core";
import type { CloudCreds } from "./cloudHistory";
import { organizationForRepository } from "./organizationKnowledge";
import type { RepositoryIdentity } from "./repositoryKnowledge";

const SECURITY_ORGANIZATION_PREFIX = "clark-desktop:security-organization:";

export interface SecurityOrganization {
  id: string;
  name: string;
  role: string;
  status: string;
}

export interface SecurityRepositoryRegistration {
  repository: {
    id: string;
    organizationId: string;
    fingerprint: string;
    canonicalRemote?: string | null;
    headOid?: string | null;
    githubManaged: boolean;
  };
  repositoryPolicy: {
    policyId: string;
    status: "active" | "paused";
    scheduleIntervalMinutes?: number | null;
    nextScanAt?: string | null;
  };
}

export type SecurityCloudScanSyncStatus =
  | "synced"
  | "already_synced"
  | "pending"
  | "failed";

export interface SecurityCloudScanSync {
  localScanId: string;
  platformScanId?: string | null;
  status: SecurityCloudScanSyncStatus;
  sealReceiptKey?: string | null;
  message?: string | null;
}

export interface SecurityCloudSyncResult {
  sealedScanCount: number;
  syncedCount: number;
  alreadySyncedCount: number;
  pendingCount: number;
  failedCount: number;
  scans: SecurityCloudScanSync[];
}

export async function inspectSecurityRepository(
  cwd: string,
): Promise<RepositoryIdentity | null> {
  return invoke<RepositoryIdentity | null>("clark_repository_inspect", { cwd });
}

export async function loadSecurityOrganizations(
  creds: CloudCreds,
): Promise<SecurityOrganization[]> {
  const organizations = await invoke<SecurityOrganization[]>(
    "desktop_security_organizations",
    { endpoint: creds.endpoint, token: creds.token },
  );
  return organizations.filter((organization) => organization.status === "active");
}

export function selectedSecurityOrganization(fingerprint: string): string | null {
  if (!fingerprint.trim()) return null;
  try {
    return (
      localStorage.getItem(`${SECURITY_ORGANIZATION_PREFIX}${fingerprint}`)
      ?? organizationForRepository(fingerprint)
    );
  } catch {
    return null;
  }
}

export function selectSecurityOrganization(
  fingerprint: string,
  organizationId: string,
): void {
  if (!fingerprint.trim() || !organizationId.trim()) return;
  try {
    localStorage.setItem(
      `${SECURITY_ORGANIZATION_PREFIX}${fingerprint}`,
      organizationId,
    );
  } catch {
    // A later open can resolve a single accessible organization again.
  }
}

export function registerSecurityRepository(
  creds: CloudCreds,
  organizationId: string,
  cwd: string,
): Promise<SecurityRepositoryRegistration> {
  return invoke<SecurityRepositoryRegistration>(
    "desktop_security_register_repository",
    {
      endpoint: creds.endpoint,
      token: creds.token,
      organizationId,
      cwd,
    },
  );
}

export function syncSecurityScans(
  creds: CloudCreds,
  apiKey: string,
  organizationId: string,
  registration: SecurityRepositoryRegistration,
  cwd: string,
): Promise<SecurityCloudSyncResult> {
  return invoke<SecurityCloudSyncResult>("desktop_security_sync_scans", {
    endpoint: creds.endpoint,
    token: creds.token,
    apiKey,
    organizationId,
    repositoryId: registration.repository.id,
    policyId: registration.repositoryPolicy.policyId,
    cwd,
  });
}
