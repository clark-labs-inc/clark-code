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
  scout: {
    id: "demo-scout-checkout-blast-radius",
    kind: "scout",
    prompt: "What breaks if the identity service becomes unavailable?",
    title: "Checkout has the clearest blast radius",
    summary:
      "Three synchronous paths depend on identity. Catalog reads degrade safely, but checkout writes have no verified fallback after the session cache expires.",
    takeaway:
      "Prioritize a checkout failover test before treating the current recovery runbook as complete.",
    diagramTitle: "Observed dependency path",
    diagram: `flowchart LR
      A["Checkout"] -->|token validation| B["Identity"]
      B -->|session claim| C["Order API"]
      B -->|cache fill| D["Session cache"]
      D -. "unverified fallback" .-> C`,
    metrics: [
      {
        label: "Blast radius",
        value: "3 paths",
        detail: "2 synchronous",
        progress: 76,
        tone: "warning",
      },
      {
        label: "Evidence",
        value: "7 receipts",
        detail: "3 source types",
        progress: 88,
        tone: "positive",
      },
      {
        label: "Confidence",
        value: "87%",
        detail: "one open gap",
        progress: 87,
        tone: "accent",
      },
    ],
    evidence: [
      {
        id: "scout-route",
        title: "Checkout calls identity synchronously",
        detail: "The accepted route map links token validation to every checkout write.",
        source: "Gateway route snapshot",
        freshness: "8 min ago",
        confidence: 96,
        status: "observed",
        tone: "positive",
      },
      {
        id: "scout-trace",
        title: "Catalog reads retain a cached path",
        detail: "Recent traces show read traffic surviving identity latency without write traffic.",
        source: "Runtime traces",
        freshness: "12 min ago",
        confidence: 84,
        status: "corroborated",
        tone: "accent",
      },
      {
        id: "scout-gap",
        title: "Checkout fallback is documented but untested",
        detail: "The runbook names a cache fallback; no accepted simulation receipt proves it.",
        source: "Runbook + simulation ledger",
        freshness: "current",
        confidence: 71,
        status: "open gap",
        tone: "warning",
      },
    ],
    stages: [
      {
        id: "scout-map",
        title: "Map authoritative dependencies",
        detail: "Gateway, identity, order API, and cache relationships reconciled.",
        status: "complete",
      },
      {
        id: "scout-reconcile",
        title: "Reconcile runtime evidence",
        detail: "Seven receipts agree on the synchronous checkout path.",
        status: "complete",
      },
      {
        id: "scout-simulate",
        title: "Simulate identity loss",
        detail: "Bounded checkout failover scenario is ready to run.",
        status: "active",
      },
      {
        id: "scout-seal",
        title: "Seal recovery conclusion",
        detail: "Blocked until the fallback path has a reproducible receipt.",
        status: "blocked",
      },
    ],
    limitation:
      "This example shows the presentation language only. A real Scout conclusion stays provisional until its cited evidence is accepted.",
  },
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
  rsi: {
    id: "demo-rsi-planning-reliability-loop",
    kind: "rsi",
    prompt:
      "Keep improving Neomobile planning reliability for 30 minutes. You may change app/planning, but do not increase memory use.",
    title: "Measuring the latest change…",
    summary:
      "Testing whether planning reliability improved without increasing memory.",
    takeaway:
      "Iteration 4 continues from the best retained code. A worse or unsafe result will be undone automatically.",
    diagramTitle: "Recursive improvement loop",
    diagram: `flowchart LR
      A["Inspect"] --> B["Propose"]
      B --> C["Code"]
      C --> D["Measure"]
      D --> E["Decide"]
      E --> A`,
    metrics: [
      {
        label: "Best safe result",
        value: "72.3",
        detail: "up from 68.2",
        progress: 72,
        tone: "positive",
      },
      {
        label: "Kept",
        value: "2",
        detail: "safe improvements",
        progress: 67,
        tone: "positive",
      },
      {
        label: "Undone",
        value: "1",
        detail: "restored automatically",
        progress: 33,
        tone: "warning",
      },
    ],
    evidence: [
      {
        id: "rsi-objective",
        title: "Planning reliability improved",
        detail: "The best retained result increased from 68.2 to 72.3.",
        source: "Independent evaluator",
        freshness: "current iteration",
        confidence: 100,
        status: "passing",
        tone: "positive",
      },
      {
        id: "rsi-memory-guardrail",
        title: "Memory guardrail is passing",
        detail: "The retained changes did not increase memory beyond the allowed limit.",
        source: "Protected check",
        freshness: "current iteration",
        confidence: 100,
        status: "passing",
        tone: "positive",
      },
      {
        id: "rsi-production-boundary",
        title: "Production unchanged",
        detail: "The improvement loop runs against the registered project checkpoint.",
        source: "Execution boundary",
        freshness: "current",
        confidence: 100,
        status: "isolated",
        tone: "positive",
      },
    ],
    stages: [
      {
        id: "rsi-inspect",
        title: "Inspect",
        detail: "Iteration 4 inspected the best retained planning code.",
        status: "complete",
      },
      {
        id: "rsi-propose",
        title: "Propose",
        detail: "One bounded stale-state improvement was selected.",
        status: "complete",
      },
      {
        id: "rsi-code",
        title: "Code",
        detail: "Clark Engineer changed only the approved planning files.",
        status: "complete",
      },
      {
        id: "rsi-measure",
        title: "Measure",
        detail: "Independent checks are measuring reliability and memory.",
        status: "active",
      },
      {
        id: "rsi-decide",
        title: "Decide",
        detail: "RSI will keep or undo the change, then repeat.",
        status: "queued",
      },
    ],
    limitation:
      "Only policy-approved files can change. Every iteration keeps a rollback checkpoint, and the loop stops at its target, time, cost, or safety boundary.",
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
