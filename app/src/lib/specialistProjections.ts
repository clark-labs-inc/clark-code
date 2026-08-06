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

export interface RsiWorldProjection {
  id: string;
  title: string;
  revision: string;
  evaluationCount: number;
  status: string;
}

export interface RsiEvaluationProjection {
  id: string;
  family: string;
  title: string;
  worldId: string;
  status: string;
  acquisitionScore: number;
  severity: number;
  coverageGap: number;
}

export interface RsiRunProjection {
  id: string;
  evaluationId: string;
  seed: number;
  driver: string;
  status: string;
  passed: boolean;
  severity: number;
  completedAt: string;
}

export interface RsiCounterexampleProjection {
  id: string;
  evaluationId: string;
  title: string;
  severity: number;
  minimizedPerturbationCount: number;
  reproducible: boolean;
}

export interface RsiOverview {
  worlds: RsiWorldProjection[];
  evaluations: RsiEvaluationProjection[];
  runs: RsiRunProjection[];
  counterexamples: RsiCounterexampleProjection[];
  familyCoverage: Record<string, number>;
  evidenceCount: number;
  lineage: {
    nodes: RsiLineageNode[];
    edges: RsiLineageEdge[];
  };
}

export interface RsiLineageNode {
  id: string;
  kind: "world" | "evaluation" | "run" | "counterexample";
  label: string;
  status: string;
}

export interface RsiLineageEdge {
  source: string;
  target: string;
  relation: string;
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

function boolean(value: unknown, label: string): boolean {
  if (typeof value !== "boolean") throw new Error(`${label} is not a boolean`);
  return value;
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

export function parseRsiOverview(value: unknown): RsiOverview {
  const root = object(value, "RSI overview");
  exactKeys(
    root,
    ["worlds", "evaluations", "runs", "counterexamples", "familyCoverage", "evidenceCount", "lineage"],
    "RSI overview",
  );
  const familyCoverage = object(root.familyCoverage, "RSI family coverage");
  const lineage = object(root.lineage, "RSI lineage");
  exactKeys(lineage, ["nodes", "edges"], "RSI lineage");
  return {
    worlds: array(root.worlds, "RSI worlds").map((entry, index) => {
      const row = object(entry, `RSI world ${index}`);
      exactKeys(
        row,
        ["id", "title", "revision", "evaluationCount", "status"],
        `RSI world ${index}`,
      );
      return {
        id: string(row.id, "world id"),
        title: string(row.title, "world title"),
        revision: string(row.revision, "world revision"),
        evaluationCount: integer(row.evaluationCount, "world evaluation count"),
        status: string(row.status, "world status"),
      };
    }),
    evaluations: array(root.evaluations, "RSI evaluations").map((entry, index) => {
      const row = object(entry, `RSI evaluation ${index}`);
      exactKeys(
        row,
        ["id", "family", "title", "worldId", "status", "acquisitionScore", "severity", "coverageGap"],
        `RSI evaluation ${index}`,
      );
      return {
        id: string(row.id, "evaluation id"),
        family: string(row.family, "evaluation family"),
        title: string(row.title, "evaluation title"),
        worldId: string(row.worldId, "evaluation world"),
        status: string(row.status, "evaluation status"),
        acquisitionScore: number(row.acquisitionScore, "evaluation acquisition score"),
        severity: number(row.severity, "evaluation severity"),
        coverageGap: number(row.coverageGap, "evaluation coverage gap"),
      };
    }),
    runs: array(root.runs, "RSI runs").map((entry, index) => {
      const row = object(entry, `RSI run ${index}`);
      exactKeys(
        row,
        ["id", "evaluationId", "seed", "driver", "status", "passed", "severity", "completedAt"],
        `RSI run ${index}`,
      );
      return {
        id: string(row.id, "RSI run id"),
        evaluationId: string(row.evaluationId, "RSI evaluation id"),
        seed: integer(row.seed, "RSI seed"),
        driver: string(row.driver, "RSI driver"),
        status: string(row.status, "RSI status"),
        passed: boolean(row.passed, "RSI pass state"),
        severity: number(row.severity, "RSI severity"),
        completedAt: string(row.completedAt, "RSI completion time"),
      };
    }),
    counterexamples: array(root.counterexamples, "RSI counterexamples").map((entry, index) => {
      const row = object(entry, `RSI counterexample ${index}`);
      exactKeys(
        row,
        ["id", "evaluationId", "title", "severity", "minimizedPerturbationCount", "reproducible"],
        `RSI counterexample ${index}`,
      );
      return {
        id: string(row.id, "counterexample id"),
        evaluationId: string(row.evaluationId, "counterexample evaluation id"),
        title: string(row.title, "counterexample title"),
        severity: number(row.severity, "counterexample severity"),
        minimizedPerturbationCount: integer(
          row.minimizedPerturbationCount,
          "counterexample perturbation count",
        ),
        reproducible: boolean(row.reproducible, "counterexample reproducibility"),
      };
    }),
    familyCoverage: Object.fromEntries(
      Object.entries(familyCoverage).map(([family, coverage]) => [
        family,
        number(coverage, `RSI ${family} coverage`),
      ]),
    ),
    evidenceCount: integer(root.evidenceCount, "RSI evidence count"),
    lineage: {
      nodes: array(lineage.nodes, "RSI lineage nodes").map((entry, index) => {
        const row = object(entry, `RSI lineage node ${index}`);
        exactKeys(row, ["id", "kind", "label", "status"], `RSI lineage node ${index}`);
        const kind = string(row.kind, "RSI lineage node kind");
        if (!["world", "evaluation", "run", "counterexample"].includes(kind)) {
          throw new Error(`RSI lineage node ${index} has an invalid kind`);
        }
        return {
          id: string(row.id, "RSI lineage node id"),
          kind: kind as RsiLineageNode["kind"],
          label: string(row.label, "RSI lineage node label"),
          status: string(row.status, "RSI lineage node status"),
        };
      }),
      edges: array(lineage.edges, "RSI lineage edges").map((entry, index) => {
        const row = object(entry, `RSI lineage edge ${index}`);
        exactKeys(row, ["source", "target", "relation"], `RSI lineage edge ${index}`);
        return {
          source: string(row.source, "RSI lineage edge source"),
          target: string(row.target, "RSI lineage edge target"),
          relation: string(row.relation, "RSI lineage edge relation"),
        };
      }),
    },
  };
}
