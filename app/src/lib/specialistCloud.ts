import type { CloudCreds } from "./cloudHistory";
import { productRequest } from "../product/productBridge";
import type { SpecialistKind } from "./specialists";
import {
  parseResearchOverview,
  type ResearchOverview,
} from "./specialistProjections";

export type {
  ResearchCampaignProjection,
  ResearchExperimentProjection,
  ResearchOverview,
  ResearchProgramProjection,
  ResearchRunProjection,
} from "./specialistProjections";

export interface SpecialistOrganization {
  id: string;
  name: string;
  role: string;
  status: string;
}

export interface SpecialistEntitlement {
  allowed: boolean;
  state: "ready" | "free" | "action_needed";
  source?: "personal" | "organization" | null;
  organizationId?: string | null;
}

export interface SecurityPosture {
  organizationId: string;
  repositoryCount: number;
  scannedRepositoryCount: number;
  staleRepositoryCount: number;
  failedOrIncompleteScanCount: number;
  openCriticalCount: number;
  openHighCount: number;
  suspectedNovelCount: number;
  confirmedNovelCount: number;
  generatedAt: string;
}

export interface SecurityFinding {
  id: string;
  repositoryId: string;
  findingKey: string;
  title: string;
  category: string;
  currentSeverity: "critical" | "high" | "medium" | "low" | "informational";
  validationState: string;
  analyticalState: string;
  workflowState: string;
  noveltyState: string;
  lastSeenAt: string;
  version: number;
}

export interface SecurityRepository {
  repositoryId: string;
  canonicalRemote?: string | null;
  serviceName?: string | null;
  latestScanStatus?: string | null;
  latestScanCreatedAt?: string | null;
  openCriticalCount: number;
  openHighCount: number;
  openMediumCount: number;
  riskScore: number;
  stale: boolean;
}

export interface SecurityScan {
  id: string;
  repositoryId: string;
  clientScanId?: string | null;
  localScanId?: string | null;
  mode: string;
  model: string;
  status: string;
  createdAt: string;
}

export interface SecurityCampaign {
  id: string;
  organizationId: string;
  title: string;
  description: string;
  status: "active" | "completed" | "canceled";
  dueAt?: string | null;
  repositoryCount: number;
  findingCount: number;
  verifiedFindingCount: number;
  version: number;
  createdAt: string;
  updatedAt: string;
  completedAt?: string | null;
}

export interface SecurityCampaignDetail {
  campaign: SecurityCampaign;
  repositories: unknown[];
  findings: unknown[];
}

export interface ScienceArtifactSegment {
  artifactId: string;
  organizationId: string;
  scopeId: string;
  logicalPath: string;
  contentType: string;
  sourceResidency: "local_only" | "remote_only" | "site_bound";
  isJournal: boolean;
  fileSizeBytes: number;
  fileSha256: string;
  segmentIndex: number;
  segmentCount: number;
  segmentSizeBytes: number;
  segmentSha256: string;
  state: "verified";
  verifiedAt: string;
  contentUri: string;
}

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function specialistOrganizations(
  _creds: CloudCreds,
): Promise<SpecialistOrganization[]> {
  if (!inTauri()) return demoOrganizations;
  return productRequest<SpecialistOrganization[]>("specialist.organizations");
}

export async function specialistCreateOrganization(
  _creds: CloudCreds,
  name: string,
  domain: string,
): Promise<SpecialistOrganization> {
  if (!inTauri()) {
    return { id: "org-demo-created", name, role: "owner", status: "active" };
  }
  return productRequest<SpecialistOrganization>("specialist.create_organization", {
    name,
    domain,
  });
}

export async function specialistEntitlement(
  _creds: CloudCreds,
  kind: SpecialistKind,
  organizationId?: string,
): Promise<SpecialistEntitlement> {
  if (!inTauri()) return { allowed: true, state: "ready", source: "personal" };
  return productRequest<SpecialistEntitlement>("specialist.entitlement", {
    specialist: kind,
    organizationId: organizationId ?? null,
  });
}

export async function specialistQuery<T>(
  _creds: CloudCreds,
  specialist: SpecialistKind,
  operation: string,
  organizationId: string,
  workspaceId?: string,
  repositoryId?: string,
): Promise<T> {
  const result = !inTauri()
    ? demoSpecialistQuery(operation)
    : await productRequest<unknown>("specialist.query", {
      specialist,
      operation,
      organizationId,
      workspaceId: workspaceId ?? null,
      repositoryId: repositoryId ?? null,
    });
  if (operation === "scientist_overview") return parseResearchOverview(result) as T;
  return result as T;
}

export async function specialistPublishOverview(
  _creds: CloudCreds,
  specialist: "scientist",
  organizationId: string,
  sequence: number,
  projection: ResearchOverview,
): Promise<ResearchOverview> {
  if (!Number.isSafeInteger(sequence) || sequence <= 0) {
    throw new Error("Specialist projection sequence must be a positive safe integer");
  }
  const validated = parseResearchOverview(projection);
  const result = !inTauri()
    ? validated
    : await productRequest<unknown>("specialist.publish", {
      specialist,
      organizationId,
      schemaVersion: 1,
      sequence,
      projection: validated,
    });
  return parseResearchOverview(result);
}

export async function specialistCreateSecurityCampaign(
  _creds: CloudCreds,
  organizationId: string,
  title: string,
  description: string,
  findingIds: string[],
): Promise<SecurityCampaignDetail> {
  if (!inTauri()) {
    const now = new Date().toISOString();
    const campaign: SecurityCampaign = {
      id: `campaign-demo-${demoCampaigns.length + 1}`,
      organizationId,
      title,
      description,
      status: "active",
      repositoryCount: new Set(
        demoFindings
          .filter((finding) => findingIds.includes(finding.id))
          .map((finding) => finding.repositoryId),
      ).size,
      findingCount: findingIds.length,
      verifiedFindingCount: 0,
      version: 1,
      createdAt: now,
      updatedAt: now,
    };
    demoCampaigns = [campaign, ...demoCampaigns];
    return { campaign, repositories: [], findings: [] };
  }
  return productRequest<SecurityCampaignDetail>("specialist.create_security_campaign", {
    organizationId,
    title,
    description,
    findingIds,
  });
}

export const demoOrganizations: SpecialistOrganization[] = [
  { id: "11111111-1111-4111-8111-111111111111", name: "Clark Labs", role: "owner", status: "active" },
];

const demoPosture: SecurityPosture = {
  organizationId: demoOrganizations[0].id,
  repositoryCount: 16,
  scannedRepositoryCount: 14,
  staleRepositoryCount: 2,
  failedOrIncompleteScanCount: 1,
  openCriticalCount: 1,
  openHighCount: 4,
  suspectedNovelCount: 2,
  confirmedNovelCount: 1,
  generatedAt: new Date().toISOString(),
};

const demoFindings: SecurityFinding[] = [
  {
    id: "finding-1",
    repositoryId: "repository-1",
    findingKey: "authz-tenant-boundary",
    title: "Tenant boundary bypass in project export",
    category: "Authorization",
    currentSeverity: "critical",
    validationState: "validated",
    analyticalState: "root_cause_confirmed",
    workflowState: "open",
    noveltyState: "known",
    lastSeenAt: new Date(Date.now() - 38 * 60_000).toISOString(),
    version: 4,
  },
  {
    id: "finding-2",
    repositoryId: "repository-2",
    findingKey: "webhook-signature-replay",
    title: "Webhook signature accepts replayed delivery",
    category: "Cryptographic verification",
    currentSeverity: "high",
    validationState: "validated",
    analyticalState: "attack_path_confirmed",
    workflowState: "in_remediation",
    noveltyState: "suspected_novel",
    lastSeenAt: new Date(Date.now() - 2 * 3_600_000).toISOString(),
    version: 2,
  },
  {
    id: "finding-3",
    repositoryId: "repository-1",
    findingKey: "archive-path-traversal",
    title: "Archive extraction crosses workspace boundary",
    category: "Path traversal",
    currentSeverity: "high",
    validationState: "needs_review",
    analyticalState: "candidate",
    workflowState: "open",
    noveltyState: "known",
    lastSeenAt: new Date(Date.now() - 6 * 3_600_000).toISOString(),
    version: 1,
  },
];

const demoRepositories: SecurityRepository[] = [
  { repositoryId: "repository-1", canonicalRemote: "github.com/example/api", serviceName: "Example API", latestScanStatus: "complete", latestScanCreatedAt: new Date(Date.now() - 38 * 60_000).toISOString(), openCriticalCount: 1, openHighCount: 2, openMediumCount: 3, riskScore: 92, stale: false },
  { repositoryId: "repository-2", canonicalRemote: "github.com/example/desktop", serviceName: "Example Desktop", latestScanStatus: "complete", latestScanCreatedAt: new Date(Date.now() - 2 * 3_600_000).toISOString(), openCriticalCount: 0, openHighCount: 2, openMediumCount: 1, riskScore: 71, stale: false },
  { repositoryId: "repository-3", canonicalRemote: "github.com/example/edge-worker", serviceName: "Example worker", latestScanStatus: "needs_attention", latestScanCreatedAt: new Date(Date.now() - 9 * 86_400_000).toISOString(), openCriticalCount: 0, openHighCount: 0, openMediumCount: 2, riskScore: 43, stale: true },
];

const demoScans: SecurityScan[] = [
  { id: "scan-1", repositoryId: "repository-1", mode: "deep", model: "GPT-5.2", status: "complete", createdAt: new Date(Date.now() - 38 * 60_000).toISOString() },
  { id: "scan-2", repositoryId: "repository-2", mode: "diff", model: "GPT-5.2", status: "running", createdAt: new Date(Date.now() - 4 * 60_000).toISOString() },
  { id: "scan-3", repositoryId: "repository-3", mode: "standard", model: "GPT-5.2", status: "needs_attention", createdAt: new Date(Date.now() - 9 * 86_400_000).toISOString() },
];

let demoCampaigns: SecurityCampaign[] = [
  {
    id: "campaign-tenant-boundary",
    organizationId: demoOrganizations[0].id,
    title: "Tenant boundary hardening",
    description: "Remediate and verify the validated tenant-isolation findings.",
    status: "active",
    repositoryCount: 1,
    findingCount: 3,
    verifiedFindingCount: 1,
    version: 2,
    createdAt: new Date(Date.now() - 3 * 86_400_000).toISOString(),
    updatedAt: new Date(Date.now() - 38 * 60_000).toISOString(),
  },
  {
    id: "campaign-webhook-authenticity",
    organizationId: demoOrganizations[0].id,
    title: "Webhook authenticity",
    description: "Close replay and signature verification gaps.",
    status: "active",
    repositoryCount: 1,
    findingCount: 2,
    verifiedFindingCount: 0,
    version: 1,
    createdAt: new Date(Date.now() - 86_400_000).toISOString(),
    updatedAt: new Date(Date.now() - 2 * 3_600_000).toISOString(),
  },
];

const demoResearchOverview: ResearchOverview = {
  programs: [{
    id: "program-product-reliability",
    title: "Product reliability discovery",
    objective: "Discover high-impact product failure modes and verify durable mitigations.",
    status: "running",
    campaignCount: 2,
    supportedClaimCount: 3,
    updatedAt: new Date(Date.now() - 8 * 60_000).toISOString(),
  }],
  campaigns: [{
    id: "campaign-adversarial-checkout",
    programId: "program-product-reliability",
    title: "Adversarial checkout campaign",
    status: "running",
    studyCount: 2,
    experimentCount: 7,
    unresolvedGateCount: 1,
  }],
  experiments: [
    {
      id: "experiment-identity-outage",
      campaignId: "campaign-adversarial-checkout",
      hypothesis: "Checkout availability depends on a synchronous identity lookup.",
      status: "accepted",
      replicationCount: 3,
      evidenceCount: 6,
      decision: "accept",
    },
    {
      id: "experiment-cache-fallback",
      campaignId: "campaign-adversarial-checkout",
      hypothesis: "A bounded identity cache preserves checkout under provider loss.",
      status: "running",
      replicationCount: 1,
      evidenceCount: 2,
    },
  ],
  runs: [{
    id: "run-cache-fallback-1",
    experimentId: "experiment-cache-fallback",
    status: "running",
    effectCount: 3,
    interruptedEffectCount: 0,
    updatedAt: new Date(Date.now() - 2 * 60_000).toISOString(),
  }],
  evidenceCount: 18,
  supportedClaimCount: 3,
};

function demoSpecialistQuery(operation: string): unknown {
  switch (operation) {
    case "security_posture": return demoPosture;
    case "security_repositories": return { data: demoRepositories, nextCursor: null };
    case "security_findings": return { data: demoFindings };
    case "security_candidates": return { data: demoFindings.filter((finding) => finding.noveltyState !== "known") };
    case "security_scans": return { data: demoScans };
    case "security_campaigns": return { data: demoCampaigns };
    case "scientist_overview": return demoResearchOverview;
    case "scientist_artifacts": return [];
    default: throw new Error(`Unsupported specialist operation: ${operation}`);
  }
}
