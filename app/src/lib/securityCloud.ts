import { invoke } from "@tauri-apps/api/core";
import type { CloudCreds } from "./cloudHistory";
import { organizationForRepository } from "./organizationKnowledge";
import type { RepositoryIdentity } from "./repositoryKnowledge";
import { accountScopedKey } from "./accountProjectStorage";
import { productRequest } from "../product/productBridge";

const SECURITY_ORGANIZATION_PREFIX = "agent-desktop:security-organization:";

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

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
  return invoke<RepositoryIdentity | null>("repository_inspect", { cwd });
}

export async function loadSecurityOrganizations(
  _creds: CloudCreds,
): Promise<SecurityOrganization[]> {
  const organizations = await productRequest<SecurityOrganization[]>(
    "security.organizations",
  );
  return organizations.filter((organization) => organization.status === "active");
}

export function selectedSecurityOrganization(
  fingerprint: string,
  scope?: string | null,
): string | null {
  if (!fingerprint.trim()) return null;
  try {
    return (
      localStorage.getItem(
        `${accountScopedKey(SECURITY_ORGANIZATION_PREFIX, scope)}${fingerprint}`,
      )
      ?? organizationForRepository(fingerprint, scope)
    );
  } catch {
    return null;
  }
}

export function selectSecurityOrganization(
  fingerprint: string,
  organizationId: string,
  scope?: string | null,
): void {
  if (!fingerprint.trim() || !organizationId.trim()) return;
  try {
    localStorage.setItem(
      `${accountScopedKey(SECURITY_ORGANIZATION_PREFIX, scope)}${fingerprint}`,
      organizationId,
    );
  } catch {
    // A later open can resolve a single accessible organization again.
  }
}

export function registerSecurityRepository(
  _creds: CloudCreds,
  organizationId: string,
  cwd: string,
): Promise<SecurityRepositoryRegistration> {
  return productRequest<SecurityRepositoryRegistration>(
    "security.register_repository",
    {
      organizationId,
      cwd,
    },
  );
}

export function syncSecurityScans(
  _creds: CloudCreds,
  organizationId: string,
  registration: SecurityRepositoryRegistration,
  cwd: string,
): Promise<SecurityCloudSyncResult> {
  return productRequest<SecurityCloudSyncResult>("security.sync_scans", {
    organizationId,
    repositoryId: registration.repository.id,
    policyId: registration.repositoryPolicy.policyId,
    cwd,
  });
}

/**
 * Publish sealed local evidence before the Security canvas reads its cloud
 * projections. Registration and scan ingestion are idempotent on the native
 * boundary, so this is safe both on workspace open and after a run settles.
 */
export async function syncSecurityInsights(
  creds: CloudCreds,
  organizationId: string,
  cwd: string,
): Promise<SecurityCloudSyncResult | null> {
  // Browser previews use the deterministic specialist projection fixtures and
  // have no native repository or sealed-scan boundary. Do not turn that valid
  // preview mode into a canvas error by attempting a Tauri command that cannot
  // exist there.
  if (!inTauri() || !organizationId.trim() || !cwd.trim()) return null;
  const repository = await inspectSecurityRepository(cwd);
  if (!repository) return null;
  const registration = await registerSecurityRepository(creds, organizationId, cwd);
  selectSecurityOrganization(repository.fingerprint, organizationId);
  return syncSecurityScans(creds, organizationId, registration, cwd);
}
