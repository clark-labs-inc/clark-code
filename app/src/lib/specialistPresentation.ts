import { isSupportedSpecialistKind, type SpecialistKind } from "./specialists";
import type { SpecialistPresentationPayload } from "../core-bridge/types";

export type SpecialistPresentationTone =
  | "neutral"
  | "accent"
  | "positive"
  | "warning"
  | "danger";

export interface SpecialistPresentationMetric {
  label: string;
  value: string;
  detail: string;
  progress: number;
  tone: SpecialistPresentationTone;
}

export interface SpecialistPresentationEvidence {
  id: string;
  title: string;
  detail: string;
  source: string;
  freshness: string;
  confidence: number;
  status: string;
  tone: SpecialistPresentationTone;
}

export interface SpecialistPresentationStage {
  id: string;
  title: string;
  detail: string;
  status: "complete" | "active" | "queued" | "blocked";
}

export interface SpecialistConversationPresentation {
  id: string;
  kind: SpecialistKind;
  prompt: string;
  title: string;
  summary: string;
  takeaway: string;
  diagram: string;
  diagramTitle: string;
  metrics: readonly SpecialistPresentationMetric[];
  evidence: readonly SpecialistPresentationEvidence[];
  stages: readonly SpecialistPresentationStage[];
  limitation: string;
}

const PRESENTATIONS: Readonly<Record<SpecialistKind, SpecialistConversationPresentation>> = {
  security: {
    id: "demo-security-archive-boundary",
    kind: "security",
    prompt: "Review the current archive-handling change for exploitable paths.",
    title: "Archive extraction can cross the workspace boundary",
    summary:
      "The new extraction path normalizes filenames after joining them to the workspace root. A crafted parent segment can therefore reach the write sink before containment is checked.",
    takeaway:
      "Move canonical containment ahead of the write and rerun both positive and negative disposable controls.",
    diagramTitle: "Validated attack path",
    diagram: `flowchart LR
      A["Archive entry"] --> B["Path join"]
      B --> C["Workspace write"]
      D["Containment guard"] -. "runs after join" .-> C
      C --> E["File outside root"]`,
    metrics: [
      {
        label: "Severity",
        value: "High",
        detail: "workspace escape",
        progress: 82,
        tone: "danger",
      },
      {
        label: "Controls",
        value: "2 / 2",
        detail: "positive + negative",
        progress: 100,
        tone: "positive",
      },
      {
        label: "Coverage",
        value: "41 / 41",
        detail: "paths reviewed",
        progress: 100,
        tone: "accent",
      },
    ],
    evidence: [
      {
        id: "security-source",
        title: "Attacker-controlled archive name",
        detail: "The source preserves parent segments until after the destination is assembled.",
        source: "Source review",
        freshness: "current diff",
        confidence: 98,
        status: "validated",
        tone: "positive",
      },
      {
        id: "security-poc",
        title: "Disposable positive control reproduced",
        detail: "The bounded copy wrote one marker beyond its assigned workspace.",
        source: "Host-issued PoC receipt",
        freshness: "4 min ago",
        confidence: 100,
        status: "reproduced",
        tone: "danger",
      },
      {
        id: "security-negative",
        title: "Safe archive remained contained",
        detail: "A distinct negative control completed without an out-of-root write.",
        source: "Host-issued control receipt",
        freshness: "4 min ago",
        confidence: 100,
        status: "control passed",
        tone: "positive",
      },
    ],
    stages: [
      {
        id: "security-model",
        title: "Threat model and inventory",
        detail: "Trust boundary and all changed paths accounted for.",
        status: "complete",
      },
      {
        id: "security-path",
        title: "Validate source → control → sink",
        detail: "Reachability and nearest guard confirmed.",
        status: "complete",
      },
      {
        id: "security-poc-stage",
        title: "Reproduce in disposable copies",
        detail: "Positive and negative controls produced distinct receipts.",
        status: "complete",
      },
      {
        id: "security-remediate",
        title: "Verify remediation",
        detail: "Queued after the containment check is moved.",
        status: "queued",
      },
    ],
    limitation:
      "Illustrative data never becomes a finding. Real Security results require complete coverage and host-issued evidence receipts.",
  },
  scientist: {
    id: "demo-scientist-latency-replication",
    kind: "scientist",
    prompt: "Does adaptive batching improve recovery latency without reducing quality?",
    title: "The latency effect is promising, not decision-grade",
    summary:
      "The preregistered treatment reduced p95 recovery latency by 18%. Quality remained inside the declared equivalence band, but the independent replication has not sealed.",
    takeaway:
      "Keep the claim provisional and run the replication on the held-out workload before adopting the scheduler.",
    diagramTitle: "Claim lineage",
    diagram: `flowchart LR
      A["Hypothesis"] --> B["Preregistered study"]
      B --> C["Observed −18% p95"]
      C --> D["Quality within band"]
      D --> E["Replication queued"]`,
    metrics: [
      {
        label: "p95 latency",
        value: "−18%",
        detail: "treatment effect",
        progress: 82,
        tone: "positive",
      },
      {
        label: "Quality delta",
        value: "−0.3%",
        detail: "inside ±1% band",
        progress: 97,
        tone: "accent",
      },
      {
        label: "Claim state",
        value: "Provisional",
        detail: "replication open",
        progress: 64,
        tone: "warning",
      },
    ],
    evidence: [
      {
        id: "scientist-prereg",
        title: "Primary outcome was fixed before execution",
        detail: "The study declared p95 recovery latency and a ±1% quality equivalence band.",
        source: "Preregistration",
        freshness: "before run",
        confidence: 100,
        status: "sealed",
        tone: "positive",
      },
      {
        id: "scientist-effect",
        title: "Treatment effect repeated across three seeds",
        detail: "All completed runs moved latency in the same direction.",
        source: "Experiment ledger",
        freshness: "16 min ago",
        confidence: 89,
        status: "observed",
        tone: "accent",
      },
      {
        id: "scientist-replication",
        title: "Independent replication remains open",
        detail: "The held-out workload is assigned but has not produced terminal evidence.",
        source: "Replication campaign",
        freshness: "queued",
        confidence: 58,
        status: "unresolved",
        tone: "warning",
      },
    ],
    stages: [
      {
        id: "scientist-preregister",
        title: "Preregister hypothesis and gates",
        detail: "Outcome, equivalence band, seeds, and stopping rule sealed.",
        status: "complete",
      },
      {
        id: "scientist-run",
        title: "Run discriminating study",
        detail: "Three treatment/control pairs completed.",
        status: "complete",
      },
      {
        id: "scientist-audit",
        title: "Audit observations and limitations",
        detail: "Interrupted effects excluded; quality band retained.",
        status: "complete",
      },
      {
        id: "scientist-replicate",
        title: "Independent replication",
        detail: "Held-out workload is waiting for capacity.",
        status: "active",
      },
    ],
    limitation:
      "The visual separates observations from claims on purpose. A provisional effect should never be presented as a supported discovery.",
  },
};

export function specialistConversationPresentation(
  kind: SpecialistKind,
): SpecialistConversationPresentation | null {
  return PRESENTATIONS[kind] ?? null;
}

export function specialistPresentationPayload(
  presentation: SpecialistConversationPresentation,
): SpecialistPresentationPayload {
  return {
    id: presentation.id,
    kind: presentation.kind,
    prompt: presentation.prompt,
    title: presentation.title,
    summary: presentation.summary,
    takeaway: presentation.takeaway,
    diagram: presentation.diagram,
    diagram_title: presentation.diagramTitle,
    metrics: [...presentation.metrics],
    evidence: [...presentation.evidence],
    stages: [...presentation.stages],
    limitation: presentation.limitation,
  };
}

export function specialistPresentationFromPayload(
  payload: SpecialistPresentationPayload,
): SpecialistConversationPresentation | null {
  if (
    !payload.id
    || !isSupportedSpecialistKind(payload.kind)
    || !payload.title
    || !payload.summary
    || !Array.isArray(payload.metrics)
    || !Array.isArray(payload.evidence)
    || !Array.isArray(payload.stages)
  ) {
    return null;
  }
  return {
    id: payload.id,
    kind: payload.kind,
    prompt: payload.prompt,
    title: payload.title,
    summary: payload.summary,
    takeaway: payload.takeaway,
    diagram: payload.diagram,
    diagramTitle: payload.diagram_title,
    metrics: payload.metrics,
    evidence: payload.evidence,
    stages: payload.stages,
    limitation: payload.limitation,
  };
}
