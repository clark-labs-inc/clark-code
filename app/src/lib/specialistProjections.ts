export interface ResearchProgramProjection {
  id: string;
  title: string;
  objective: string;
  status: string;
  campaignCount: number;
  supportedClaimCount: number;
  updatedAt: string;
}

export interface ResearchCampaignProjection {
  id: string;
  programId: string;
  title: string;
  status: string;
  studyCount: number;
  experimentCount: number;
  unresolvedGateCount: number;
}

export interface ResearchExperimentProjection {
  id: string;
  campaignId: string;
  hypothesis: string;
  status: string;
  replicationCount: number;
  evidenceCount: number;
  decision?: string | null;
}

export interface ResearchRunProjection {
  id: string;
  experimentId: string;
  status: string;
  effectCount: number;
  interruptedEffectCount: number;
  updatedAt: string;
}

export interface ResearchOverview {
  programs: ResearchProgramProjection[];
  campaigns: ResearchCampaignProjection[];
  experiments: ResearchExperimentProjection[];
  runs: ResearchRunProjection[];
  evidenceCount: number;
  supportedClaimCount: number;
}

type JsonObject = Record<string, unknown>;

function object(value: unknown, label: string): JsonObject {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} is not an object`);
  }
  return value as JsonObject;
}

function exactKeys(value: JsonObject, keys: readonly string[], label: string): void {
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    throw new Error(`${label} does not match the v1 projection schema`);
  }
}

function string(value: unknown, label: string): string {
  if (typeof value !== "string") throw new Error(`${label} is not a string`);
  return value;
}

function number(value: unknown, label: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new Error(`${label} is not a finite number`);
  }
  return value;
}

function integer(value: unknown, label: string): number {
  const parsed = number(value, label);
  if (!Number.isSafeInteger(parsed) || parsed < 0) {
    throw new Error(`${label} is not a non-negative safe integer`);
  }
  return parsed;
}

function array(value: unknown, label: string): unknown[] {
  if (!Array.isArray(value)) throw new Error(`${label} is not an array`);
  return value;
}

function optionalString(value: unknown, label: string): string | null | undefined {
  if (value === undefined || value === null) return value;
  return string(value, label);
}

export function parseResearchOverview(value: unknown): ResearchOverview {
  const root = object(value, "Scientist overview");
  exactKeys(
    root,
    ["programs", "campaigns", "experiments", "runs", "evidenceCount", "supportedClaimCount"],
    "Scientist overview",
  );
  return {
    programs: array(root.programs, "Scientist programs").map((entry, index) => {
      const row = object(entry, `Scientist program ${index}`);
      exactKeys(
        row,
        ["id", "title", "objective", "status", "campaignCount", "supportedClaimCount", "updatedAt"],
        `Scientist program ${index}`,
      );
      return {
        id: string(row.id, "program id"),
        title: string(row.title, "program title"),
        objective: string(row.objective, "program objective"),
        status: string(row.status, "program status"),
        campaignCount: integer(row.campaignCount, "program campaign count"),
        supportedClaimCount: integer(row.supportedClaimCount, "program supported claim count"),
        updatedAt: string(row.updatedAt, "program update time"),
      };
    }),
    campaigns: array(root.campaigns, "Scientist campaigns").map((entry, index) => {
      const row = object(entry, `Scientist campaign ${index}`);
      exactKeys(
        row,
        ["id", "programId", "title", "status", "studyCount", "experimentCount", "unresolvedGateCount"],
        `Scientist campaign ${index}`,
      );
      return {
        id: string(row.id, "campaign id"),
        programId: string(row.programId, "campaign program id"),
        title: string(row.title, "campaign title"),
        status: string(row.status, "campaign status"),
        studyCount: integer(row.studyCount, "campaign study count"),
        experimentCount: integer(row.experimentCount, "campaign experiment count"),
        unresolvedGateCount: integer(row.unresolvedGateCount, "campaign unresolved gate count"),
      };
    }),
    experiments: array(root.experiments, "Scientist experiments").map((entry, index) => {
      const row = object(entry, `Scientist experiment ${index}`);
      const required = ["id", "campaignId", "hypothesis", "status", "replicationCount", "evidenceCount"];
      const allowed = new Set([...required, "decision"]);
      if (Object.keys(row).some((key) => !allowed.has(key)) || required.some((key) => !(key in row))) {
        throw new Error(`Scientist experiment ${index} does not match the v1 projection schema`);
      }
      return {
        id: string(row.id, "experiment id"),
        campaignId: string(row.campaignId, "experiment campaign id"),
        hypothesis: string(row.hypothesis, "experiment hypothesis"),
        status: string(row.status, "experiment status"),
        replicationCount: integer(row.replicationCount, "experiment replication count"),
        evidenceCount: integer(row.evidenceCount, "experiment evidence count"),
        decision: optionalString(row.decision, "experiment decision"),
      };
    }),
    runs: array(root.runs, "Scientist runs").map((entry, index) => {
      const row = object(entry, `Scientist run ${index}`);
      exactKeys(
        row,
        ["id", "experimentId", "status", "effectCount", "interruptedEffectCount", "updatedAt"],
        `Scientist run ${index}`,
      );
      return {
        id: string(row.id, "run id"),
        experimentId: string(row.experimentId, "run experiment id"),
        status: string(row.status, "run status"),
        effectCount: integer(row.effectCount, "run effect count"),
        interruptedEffectCount: integer(row.interruptedEffectCount, "run interrupted effect count"),
        updatedAt: string(row.updatedAt, "run update time"),
      };
    }),
    evidenceCount: integer(root.evidenceCount, "Scientist evidence count"),
    supportedClaimCount: integer(root.supportedClaimCount, "Scientist supported claim count"),
  };
}
