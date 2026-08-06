import { invoke } from "@tauri-apps/api/core";
import type { CloudCreds } from "./cloudHistory";
import type { SpecialistKind } from "./specialists";
import {
  parseResearchOverview,
  parseRsiOverview,
  type ResearchOverview,
  type RsiOverview,
} from "./specialistProjections";

export type {
  ResearchCampaignProjection,
  ResearchExperimentProjection,
  ResearchOverview,
  ResearchProgramProjection,
  ResearchRunProjection,
  RsiCounterexampleProjection,
  RsiEvaluationProjection,
  RsiOverview,
  RsiRunProjection,
  RsiWorldProjection,
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

export interface ScoutWorkspace {
  id: string;
  organization_id: string;
  stable_key: string;
  display_name: string;
  status: string;
  latest_change_sequence: number;
  source_count: number;
  active_machine_count: number;
  run_count: number;
  simulation_count: number;
  updated_at_ms: number;
}

export interface ScoutSnapshotEntry {
  object_kind: "entity" | "edge" | "claim" | "coverage";
  object_id: string;
  accepted_at_ms: number;
  event: {
    classification: string;
    fact: {
      subject: unknown;
      attributes: Record<string, unknown>;
    };
  };
}

export interface ScoutChange {
  sequence: number;
  event_type: string;
  occurred_at_ms: number;
  payload: Record<string, unknown>;
}

export interface ScoutSimulation {
  id: string;
  stable_key: string;
  version: number;
  name: string;
  status: string;
  membership_count: number;
  created_at_ms: number;
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
  return invoke<SpecialistOrganization[]>("desktop_specialist_organizations");
}

export async function specialistEntitlement(
  _creds: CloudCreds,
  kind: SpecialistKind,
  organizationId?: string,
): Promise<SpecialistEntitlement> {
  if (!inTauri()) return { allowed: true, state: "ready", source: "personal" };
  return invoke<SpecialistEntitlement>("desktop_specialist_entitlement", {
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
    : await invoke<unknown>("desktop_specialist_query", {
      specialist,
      operation,
      organizationId,
      workspaceId: workspaceId ?? null,
      repositoryId: repositoryId ?? null,
    });
  if (operation === "scientist_overview") return parseResearchOverview(result) as T;
  if (operation === "rsi_overview") return parseRsiOverview(result) as T;
  return result as T;
}

export async function specialistPublishOverview(
  _creds: CloudCreds,
  specialist: "scientist" | "rsi",
  organizationId: string,
  sequence: number,
  projection: ResearchOverview | RsiOverview,
): Promise<ResearchOverview | RsiOverview> {
  if (!Number.isSafeInteger(sequence) || sequence <= 0) {
    throw new Error("Specialist projection sequence must be a positive safe integer");
  }
  const validated = specialist === "scientist"
    ? parseResearchOverview(projection)
    : parseRsiOverview(projection);
  const result = !inTauri()
    ? validated
    : await invoke<unknown>("desktop_specialist_publish", {
      specialist,
      organizationId,
      schemaVersion: 1,
      sequence,
      projection: validated,
    });
  return specialist === "scientist"
    ? parseResearchOverview(result)
    : parseRsiOverview(result);
}

/** A workspace created through `POST /cli/scout/workspaces`. The create
 * endpoint returns a minimal camelCase shape (id, displayName, status); the
 * full workspace with snapshot/sequence counts arrives on the next
 * `scout_workspaces` list refresh. */
export interface ScoutCreatedWorkspace {
  id: string;
  displayName?: string;
  status?: string;
}

/** Create a Scout cartography workspace for an organization. This reaches the
 * same `POST /cli/scout/workspaces` contract `clark-cli` uses with
 * `--create-workspace`, so the first Scout run for an organization that has no
 * workspace yet can enroll and upload evidence instead of failing with
 * "backend is not host-configured". */
export async function specialistCreateWorkspace(
  _creds: CloudCreds,
  organizationId: string,
  displayName: string,
): Promise<ScoutCreatedWorkspace> {
  if (!inTauri()) {
    return {
      id: "ws-demo-created",
      displayName,
      status: "active",
    };
  }
  return invoke<ScoutCreatedWorkspace>("desktop_specialist_create_workspace", {
    organizationId,
    displayName,
  });
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
  return invoke<SecurityCampaignDetail>("desktop_specialist_create_security_campaign", {
    organizationId,
    title,
    description,
    findingIds,
  });
}

export const demoOrganizations: SpecialistOrganization[] = [
  { id: "11111111-1111-4111-8111-111111111111", name: "Clark Labs", role: "owner", status: "active" },
];

const demoWorkspaces: ScoutWorkspace[] = [{
  id: "22222222-2222-4222-8222-222222222222",
  organization_id: demoOrganizations[0].id,
  stable_key: "production",
  display_name: "Production estate",
  status: "active",
  latest_change_sequence: 1842,
  source_count: 7,
  active_machine_count: 14,
  run_count: 28,
  simulation_count: 4,
  updated_at_ms: Date.now() - 12 * 60_000,
}];

const demoEntries: ScoutSnapshotEntry[] = [
  {
    object_kind: "entity",
    object_id: "api-gateway",
    accepted_at_ms: Date.now() - 22_000,
    event: {
      classification: "internal",
      fact: { subject: { type: "entity" }, attributes: { name: "Public API", kind: "gateway", provider: "AWS" } },
    },
  },
  {
    object_kind: "entity",
    object_id: "identity-service",
    accepted_at_ms: Date.now() - 31_000,
    event: {
      classification: "confidential",
      fact: { subject: { type: "entity" }, attributes: { name: "Identity service", kind: "service", owner: "Platform" } },
    },
  },
  {
    object_kind: "entity",
    object_id: "customer-db",
    accepted_at_ms: Date.now() - 42_000,
    event: {
      classification: "restricted",
      fact: { subject: { type: "entity" }, attributes: { name: "Customer database", kind: "database", region: "us-west-2" } },
    },
  },
  {
    object_kind: "edge",
    object_id: "gateway-to-identity",
    accepted_at_ms: Date.now() - 20_000,
    event: {
      classification: "internal",
      fact: { subject: { type: "edge" }, attributes: { name: "Routes authentication", protocol: "HTTPS" } },
    },
  },
];

const demoChanges: ScoutChange[] = [
  { sequence: 1842, event_type: "batch_accepted", occurred_at_ms: Date.now() - 12 * 60_000, payload: { source: "AWS", accepted: 38 } },
  { sequence: 1841, event_type: "simulation_overlay_published", occurred_at_ms: Date.now() - 3_600_000, payload: { name: "Identity service outage" } },
  { sequence: 1840, event_type: "batch_accepted", occurred_at_ms: Date.now() - 8_400_000, payload: { source: "GitHub", accepted: 12 } },
];

const demoSimulations: ScoutSimulation[] = [
  { id: "sim-1", stable_key: "identity-outage", version: 3, name: "Identity service outage", status: "complete", membership_count: 23, created_at_ms: Date.now() - 3_600_000 },
  { id: "sim-2", stable_key: "region-loss", version: 1, name: "us-west-2 regional loss", status: "ready", membership_count: 41, created_at_ms: Date.now() - 86_400_000 },
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
  { repositoryId: "repository-1", canonicalRemote: "github.com/clark-ai/clark", serviceName: "Clark API", latestScanStatus: "complete", latestScanCreatedAt: new Date(Date.now() - 38 * 60_000).toISOString(), openCriticalCount: 1, openHighCount: 2, openMediumCount: 3, riskScore: 92, stale: false },
  { repositoryId: "repository-2", canonicalRemote: "github.com/clark-ai/clark-desktop", serviceName: "Clark Desktop", latestScanStatus: "complete", latestScanCreatedAt: new Date(Date.now() - 2 * 3_600_000).toISOString(), openCriticalCount: 0, openHighCount: 2, openMediumCount: 1, riskScore: 71, stale: false },
  { repositoryId: "repository-3", canonicalRemote: "github.com/clark-ai/edge-worker", serviceName: "Edge worker", latestScanStatus: "needs_attention", latestScanCreatedAt: new Date(Date.now() - 9 * 86_400_000).toISOString(), openCriticalCount: 0, openHighCount: 0, openMediumCount: 2, riskScore: 43, stale: true },
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

const demoRsiOverview: RsiOverview = {
  worlds: [{
    id: "checkout",
    title: "Checkout reliability world",
    revision: "intent-sha256:demo",
    evaluationCount: 2,
    status: "running",
  }],
  evaluations: [
    {
      id: "identity-outage",
      family: "dependency-failure",
      title: "Identity provider becomes unavailable",
      worldId: "checkout",
      status: "completed",
      acquisitionScore: 3.42,
      severity: 0.9,
      coverageGap: 0.8,
    },
    {
      id: "stale-session-race",
      family: "concurrency",
      title: "Stale session wins a checkout race",
      worldId: "checkout",
      status: "queued",
      acquisitionScore: 3.18,
      severity: 0.85,
      coverageGap: 0.92,
    },
  ],
  runs: [{
    id: "simulation-run-17",
    evaluationId: "identity-outage",
    seed: 17,
    driver: "deterministic-state-v1",
    status: "completed",
    passed: false,
    severity: 0.9,
    completedAt: new Date(Date.now() - 6 * 60_000).toISOString(),
  }],
  counterexamples: [{
    id: "counterexample-identity-outage",
    evaluationId: "identity-outage",
    title: "Checkout success collapses after identity loss",
    severity: 0.9,
    minimizedPerturbationCount: 2,
    reproducible: true,
  }],
  familyCoverage: {
    "dependency-failure": 0.82,
    concurrency: 0.37,
    degradation: 0.61,
  },
  evidenceCount: 7,
  lineage: {
    nodes: [
      { id: "checkout", kind: "world", label: "Checkout reliability world", status: "running" },
      { id: "identity-outage", kind: "evaluation", label: "Identity provider becomes unavailable", status: "completed" },
      { id: "simulation-run-17", kind: "run", label: "seed 17", status: "completed" },
      { id: "counterexample-identity-outage", kind: "counterexample", label: "Checkout success collapses after identity loss", status: "reproducible" },
    ],
    edges: [
      { source: "checkout", target: "identity-outage", relation: "evaluates" },
      { source: "identity-outage", target: "simulation-run-17", relation: "executed_as" },
      { source: "simulation-run-17", target: "counterexample-identity-outage", relation: "found" },
    ],
  },
};

function demoSpecialistQuery(operation: string): unknown {
  switch (operation) {
    case "scout_workspaces": return demoWorkspaces;
    case "scout_snapshot": return { entries: demoEntries, next_cursor: null };
    case "scout_changes": return { changes: demoChanges, next_after_sequence: 1842 };
    case "scout_simulations": return demoSimulations;
    case "security_posture": return demoPosture;
    case "security_repositories": return { data: demoRepositories, nextCursor: null };
    case "security_findings": return { data: demoFindings };
    case "security_candidates": return { data: demoFindings.filter((finding) => finding.noveltyState !== "known") };
    case "security_scans": return { data: demoScans };
    case "security_campaigns": return { data: demoCampaigns };
    case "scientist_overview": return demoResearchOverview;
    case "scientist_artifacts": return [];
    case "rsi_overview": return demoRsiOverview;
    case "rsi_artifacts": return [];
    default: throw new Error(`Unsupported specialist operation: ${operation}`);
  }
}
