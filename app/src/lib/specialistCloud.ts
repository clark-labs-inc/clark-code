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

export const demoOrganizations: SpecialistOrganization[] = [
  { id: "11111111-1111-4111-8111-111111111111", name: "Clark Labs", role: "owner", status: "active" },
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
    case "scientist_overview": return demoResearchOverview;
    case "scientist_artifacts": return [];
    default: throw new Error(`Unsupported specialist operation: ${operation}`);
  }
}
