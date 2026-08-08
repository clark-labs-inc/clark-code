import type { CloudCreds } from "./cloudHistory";
import { desktopHostId } from "./desktopHost";
import { accountScopedKey } from "./accountProjectStorage";
import { productRequest } from "../product/productBridge";

const REPOSITORY_OPT_IN_PREFIX = "agent-desktop:organization-knowledge-repository:";
const ORGANIZATION_KNOWLEDGE_SETTING_EVENT = "agent-desktop:organization-knowledge-setting";

export interface OrganizationKnowledgeOrganization {
  organization_id: string;
  name: string;
  role: string;
}

export interface OrganizationKnowledgeStatus {
  organizations: OrganizationKnowledgeOrganization[];
  contribution_mode: "explicit_opt_in";
}

export async function loadOrganizationKnowledgeStatus(
  _creds: CloudCreds,
): Promise<OrganizationKnowledgeStatus> {
  return productRequest<OrganizationKnowledgeStatus>("organization_knowledge.status");
}

/** The selected organization for this exact repository, or null when private. */
export function organizationForRepository(
  fingerprint: string,
  scope?: string | null,
): string | null {
  if (!fingerprint.trim()) return null;
  try {
    return localStorage.getItem(
      `${accountScopedKey(REPOSITORY_OPT_IN_PREFIX, scope)}${fingerprint}`,
    );
  } catch {
    return null;
  }
}

export function setOrganizationForRepository(
  fingerprint: string,
  organizationId: string | null,
  scope?: string | null,
): void {
  if (!fingerprint.trim()) return;
  try {
    const key = `${accountScopedKey(REPOSITORY_OPT_IN_PREFIX, scope)}${fingerprint}`;
    if (organizationId) localStorage.setItem(key, organizationId);
    else localStorage.removeItem(key);
    window.dispatchEvent(new CustomEvent(ORGANIZATION_KNOWLEDGE_SETTING_EVENT, {
      detail: { fingerprint, organizationId },
    }));
  } catch {
    // A hardened preview may not provide local storage; private is the fallback.
  }
}

export function uploadOrganizationRepositoryBatch<T>(
  _creds: CloudCreds,
  organizationId: string,
  batch: unknown,
): Promise<T> {
  return productRequest<T>("organization_knowledge.repository_sync", {
    organizationId,
    hostId: desktopHostId(),
    batch,
  });
}
