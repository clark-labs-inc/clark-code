// Hand-mirrored from the `agent-core` Rust types; keep them in sync with
// crates/agent-core/src/{domain,projection,provider}.rs.

import { quarantineSnapshotProviderOutput } from "./providerOutputQuarantine";

export type Role = "user" | "agent" | "system";
export type MessagePhase = "commentary" | "final_answer";

export type ContentBlock =
  | { type: "text"; text: string }
  | { type: "thinking"; text: string }
  | { type: "image"; mime_type: string; data: string; uri?: string }
  | { type: "audio"; mime_type: string; data: string }
  | { type: "resource"; uri: string; mime_type?: string; text?: string; data?: string }
  | { type: "resource_link"; uri: string; name?: string }
  | { type: "skill_reference"; id: string; revision: string; name: string };

export type SpecialistPresentationTone =
  | "neutral"
  | "accent"
  | "positive"
  | "warning"
  | "danger";

export interface SpecialistPresentationMetricPayload {
  label: string;
  value: string;
  detail: string;
  progress: number;
  tone: SpecialistPresentationTone;
}

export interface SpecialistPresentationEvidencePayload {
  id: string;
  title: string;
  detail: string;
  source: string;
  freshness: string;
  confidence: number;
  status: string;
  tone: SpecialistPresentationTone;
}

export interface SpecialistPresentationStagePayload {
  id: string;
  title: string;
  detail: string;
  status: "complete" | "active" | "queued" | "blocked";
}

/** Structured specialist output rendered inline in the conversation rail. */
export interface SpecialistPresentationPayload {
  id: string;
  kind: string;
  prompt: string;
  title: string;
  summary: string;
  takeaway: string;
  diagram: string;
  diagram_title: string;
  metrics: SpecialistPresentationMetricPayload[];
  evidence: SpecialistPresentationEvidencePayload[];
  stages: SpecialistPresentationStagePayload[];
  limitation: string;
}

export type ToolKind =
  | "read" | "edit" | "delete" | "move"
  | "search" | "execute" | "think" | "fetch" | "research"
  | "view_image" | "generate_image" | "other";

export type ToolStatus = "pending" | "in_progress" | "completed" | "cancelled" | "failed";

export interface FsLocation {
  path: string;
  line?: number;
}

export interface ToolProgressStep {
  id: string;
  title: string;
  status: ToolStatus;
  summary?: string;
}

export interface ToolProgressPhase {
  id: string;
  title: string;
  status: ToolStatus;
  summary?: string;
  steps: ToolProgressStep[];
}

export interface ToolProgressAgent {
  id: string;
  label: string;
  status: ToolStatus;
  activity?: string;
  summary?: string;
}

/** Presentation-safe progress for a long-running delegated tool call. */
export interface ToolCallProgress {
  revision: number;
  status: ToolStatus;
  latest_activity?: string;
  phases: ToolProgressPhase[];
  agents: ToolProgressAgent[];
}

export interface ToolCall {
  id: string;
  tool_name?: string;
  title: string;
  kind: ToolKind;
  status: ToolStatus;
  locations: FsLocation[];
  content: ContentBlock[];
  raw_input?: unknown;
  progress?: ToolCallProgress;
}

export type ResumeItem =
  | { item: "message"; role: Role; blocks: ContentBlock[] }
  | {
      item: "tool_call";
      id: string;
      tool_name?: string;
      title: string;
      kind: ToolKind;
      status: ToolStatus;
      locations: FsLocation[];
      arguments?: unknown;
      content: ContentBlock[];
    }
  | { item: "goal"; goal: GoalState }
  | { item: "proposed_plan"; plan: ProposedPlan };

export interface ResumeTranscript {
  items: ResumeItem[];
  truncated: boolean;
}

export interface ModelContextCheckpoint {
  transcript: ResumeTranscript;
  /** Visible timeline length when the replacement context was installed. */
  timeline_index: number;
}

export type ChecklistStatus = "pending" | "in_progress" | "completed";

export interface ChecklistStep {
  title: string;
  status: ChecklistStatus;
  priority?: string;
}

export interface ExecutionChecklist {
  steps: ChecklistStep[];
  revision: number;
}

export type ProposedPlanStatus = "awaiting_decision" | "approved" | "superseded";

export interface PlanContextRevision {
  context_kind: string;
  organization_id?: string;
  workspace_id?: string;
  query: string;
  effective_at_ms: number;
  known_at_ms: number;
  selector_sha256: string;
}

export interface ProposedPlan {
  id: string;
  revision: number;
  markdown: string;
  status: ProposedPlanStatus;
  context_revisions?: PlanContextRevision[];
}

export type PermissionOptionKind =
  | "allow_once" | "allow_always" | "reject_once" | "reject_always";

export interface PermissionOption {
  id: string;
  label: string;
  kind: PermissionOptionKind;
}

export interface PermissionRequest {
  id: string;
  session: string;
  tool_call?: string;
  title: string;
  options: PermissionOption[];
  /** What the action does — a shell command or file path — shown for review. */
  detail?: string;
  /** Action classification: shell risk, external tool, billed image, or plan gate. */
  risk?: string;
  /** Why it was flagged ("recursive delete"). */
  reason?: string;
}

export type WorkspaceSurfaceKind = "browser" | "terminal" | "files" | "website";

export interface WorkspaceFocus {
  surface: WorkspaceSurfaceKind;
  path?: string;
  url?: string;
  is_dir?: boolean;
  tool_call?: string;
}

export type FanOutStatus = "queued" | "running" | "done" | "failed";

export interface FanOutAgent {
  id: string;
  label: string;
  status: FanOutStatus;
  /** Full backend-authored task objective shown in the inspector. */
  objective?: string;
  /** Latest public progress update; never hidden model reasoning. */
  activity?: string;
  /** Final public result or failure summary. */
  result?: string;
  attempt?: number;
  started_at_ms?: number;
  updated_at_ms?: number;
}

/** Live parallel fan-out (a `subagent_map` spread across child agents). */
export interface FanOut {
  title: string;
  total: number;
  done: number;
  running: number;
  agents: FanOutAgent[];
}

export type ArtifactKind =
  | "file" | "image" | "pdf" | "office"
  | "slides" | "media" | "video" | "website" | "diff" | "search_results" | "other";

export interface Artifact {
  id: string;
  title: string;
  kind: ArtifactKind;
  mime_type?: string;
  uri?: string;
  tool_call?: string;
}

export type RunStatus =
  | "queued" | "running" | "awaiting_input" | "done" | "cancelled" | "failed";

export type RunFailureKind =
  | "session_expired"
  | "platform_key_rejected"
  | "access_scope_required"
  | "provider_error"
  | "rate_limited"
  | "transport_error"
  | "context_overflow"
  | "insufficient_credits"
  | "tool_fatal"
  | "local_state"
  | "inconsistent_tool_history"
  | "iteration_limit"
  | "runtime_interrupted"
  | "verification_incomplete"
  | "empty_response";

/** Aggregated model usage for one run (the local coding loop surfaces it). */
export interface RunUsage {
  input_tokens: number;
  output_tokens: number;
  /** Prompt size of the last model call — the live context footprint. */
  context_tokens: number;
  cost_usd?: number;
  /** The engine's auto-compaction threshold in tokens, when known — the
   *  denominator for an honest context meter. */
  context_limit?: number;
}

export interface RunOutcome {
  status: RunStatus;
  stop_reason?: string;
  error?: string;
  failure_kind?: RunFailureKind;
  usage?: RunUsage;
  /** Runtime-derived receipt for the root execution tree. */
  execution?: {
    execution_id: string;
    root_path: string;
    attempts: number;
    recoveries: number;
    child_executions: number;
    completed_children: number;
    failed_children: number;
    weighted_tokens: number;
    cost_usd: number;
    changed_paths: string[];
    completed_tools: string[];
    failed_tools: string[];
  };
}

export interface RunView {
  id: string;
  status: RunStatus;
  /** Latest cumulative usage, available before the run finishes. */
  usage?: RunUsage;
  outcome?: RunOutcome;
  /** Pre-run working-tree checkpoint used as a change-tracking baseline. */
  checkpoint?: string;
}

export type ProviderIncidentCategory =
  | "timeout"
  | "rate_limit"
  | "upstream_unavailable"
  | "connection_lost";

export type ProviderIncidentStatus =
  | "observed"
  | "retrying"
  | "recovered"
  | "failed"
  | "interrupted";
export type ProviderFailureClass = "transient_transport" | "rate_limited";
export type ProviderIncidentScope =
  | "model_request"
  | "provider_event_stream"
  | "provider_process"
  | "cloud_history_sync"
  | "tool_execution_host";

export interface ProviderRetryCounts {
  transient: number;
  rate_limit: number;
  authentication: number;
}

export interface ProviderRequestDiagnostics {
  idempotency_key: string;
  provider_request_id?: string;
  attempts: number;
  max_attempts: number;
  retries: ProviderRetryCounts;
  output_started: boolean;
  started_at_ms: number;
}

export interface ExecutionBoundaryReceipt {
  execution_id: string;
  attempt_sequence: number;
  event_sequence: number;
  transcript_commit_id: string;
  completed_tools: number;
  last_completed_tool_id?: string;
  last_completed_tool_name?: string;
  baseline_checkpoint_id?: string;
}

export interface ExecutionRecovery {
  attempt: number;
  boundary: ExecutionBoundaryReceipt;
  started_at_ms: number;
}

/** Durable provider incident, independent from optional execution recovery. */
export interface ProviderIncident {
  id: string;
  status: ProviderIncidentStatus;
  scope: ProviderIncidentScope;
  failure_class: ProviderFailureClass;
  category: ProviderIncidentCategory;
  message: string;
  detail: string;
  model: string;
  provider_route: string;
  provider_status?: number;
  provider_error_type?: string;
  request: ProviderRequestDiagnostics;
  execution_recovery?: ExecutionRecovery;
  observed_at_ms: number;
  updated_at_ms: number;
  completed_at_ms?: number;
}

export type GoalStatus = "active" | "blocked" | "complete";

/** Provider-owned receipt for a standing goal that can span many runs. */
export interface GoalState {
  id: string;
  objective: string;
  status: GoalStatus;
  run?: string;
  tokens_used: number;
  time_used_seconds: number;
  continuations: number;
  updated_at_ms: number;
  blocker_reason?: string;
}

export type TimelineItem =
  | { item: "message"; run: string; role: Role; blocks: ContentBlock[]; phase?: MessagePhase }
  | { item: "specialist_presentation"; run: string; presentation: SpecialistPresentationPayload }
  | { item: "tool_call"; id: string; run?: string }
  | { item: "artifact"; id: string }
  | { item: "provider_incident"; run: string; id: string }
  | {
      item: "execution_checklist";
      run?: string;
      checklist?: ExecutionChecklist;
      explanation?: string;
    }
  | { item: "proposed_plan"; run: string; plan: ProposedPlan };

export interface Snapshot {
  /** Local outbox has durable events not yet acknowledged by product cloud. */
  sync_pending?: boolean;
  /** Opaque device-local journal cursor used by the native bridge to
   * checkpoint exactly the event prefix represented by this snapshot. */
  history_checkpoint?: number;
  session?: string;
  /** The host accepted a prompt but the provider hasn't allocated its run yet
   * (attachment upload / connect handshake in flight). Keeps a "working now"
   * row animated through the upload; cleared when `RunStarted` lands. */
  starting?: boolean;
  runs: Record<string, RunView>;
  /** Absolute index of timeline[0] when older immutable pages are archived. */
  timeline_offset?: number;
  timeline: TimelineItem[];
  model_context_checkpoint?: ModelContextCheckpoint;
  tool_calls: Record<string, ToolCall>;
  execution_checklist?: ExecutionChecklist;
  proposed_plan?: ProposedPlan;
  goal?: GoalState;
  pending_permission?: PermissionRequest;
  artifacts: Artifact[];
  focus?: WorkspaceFocus;
  fan_out?: FanOut;
  provider_incidents: Record<string, ProviderIncident>;
}

/** Immutable cloud transcript page. Pages are loaded into a bounded history
 * viewport and never merged into the live projection. */
export interface TranscriptPage {
  startIndex: number;
  items: TimelineItem[];
  toolCalls?: Record<string, ToolCall>;
  artifacts?: Artifact[];
  providerIncidents?: Record<string, ProviderIncident>;
}

export function emptySnapshot(): Snapshot {
  return { runs: {}, timeline: [], tool_calls: {}, artifacts: [], provider_incidents: {}, starting: false };
}

/** Legacy/cloud/native payload shape before it crosses into application state. */
export type WireSnapshot = Omit<Snapshot, "provider_incidents"> & {
  provider_incidents?: Record<string, ProviderIncident>;
};

export function normalizeSnapshot(snapshot: WireSnapshot): Snapshot {
  return quarantineSnapshotProviderOutput({
    ...snapshot,
    provider_incidents: snapshot.provider_incidents ?? {},
  });
}

export interface ProviderCapabilities {
  streaming: boolean;
  permissions: boolean;
  fs: boolean;
  terminal: boolean;
  load_session: boolean;
  /** Raw attachment families this provider can ingest without losing bytes. */
  attachment_kinds?: AttachmentKind[];
  modes: string[];
  collaboration_modes: CollaborationMode[];
}

export type AttachmentKind = "text" | "image" | "audio" | "pdf" | "docx" | "binary";

export type CollaborationMode = "default" | "plan";

export interface ProviderInfo {
  id: string;
  label: string;
  capabilities: ProviderCapabilities;
  /** Native execution adapter selected by a product surface, not a user-facing
   * environment choice. */
  internal?: boolean;
}

export interface SpecialistProjectionPublished {
  specialist: "scientist" | "rsi";
  organizationId: string;
  sequence: number;
  programId?: string;
}

export interface SpecialistCatalogAttestation {
  schemaVersion: number;
  catalogVersion: string;
  catalogSha256: string;
  trust: {
    source: string;
    requiresSignedReleaseBinary: boolean;
  };
  manifests: unknown[];
}

export interface SessionEnvironment {
  checkout_root?: string;
  repository_root?: string;
  workspace_roots: string[];
  docs_root?: string;
  remote: boolean;
}

export interface Session {
  id: string;
  provider: string;
  capabilities: ProviderCapabilities;
  mode?: string;
  collaboration_mode: CollaborationMode;
  environment?: SessionEnvironment;
}

export type PlanImplementationContext = "current" | "fresh";

export type PlanDecision =
  | { action: "implement"; context: PlanImplementationContext }
  | { action: "continue_planning"; feedback?: string };

export type ClientResponse =
  | {
      kind: "permission";
      request: string;
      option: string;
      feedback?: string;
    }
  | {
      kind: "plan_decision";
      plan_id: string;
      decision: PlanDecision;
    };

/** One per-fact memory file under `<cwd>/.agent/memory/`. */
export interface MemoryFactView {
  file: string;
  name?: string | null;
  description?: string | null;
  kind?: string | null;
  body: string;
}

/** The per-repository memory for one project folder (index + fact files). */
export interface MemoryOverview {
  /** Absolute path to `<cwd>/.agent/memory`. */
  dir: string;
  /** Whether a `MEMORY.md` index has been written. */
  exists: boolean;
  /** Contents of the always-loaded `MEMORY.md` index, if present. */
  index?: string | null;
  /** Per-fact memory files (newest first). */
  facts: MemoryFactView[];
}

export type SecurityScanMode = "standard" | "diff" | "deep";
export type SecuritySeverity = "low" | "medium" | "high" | "critical";
export type SecurityConfidence = "low" | "medium" | "high";

export interface SecurityScanRecord {
  path: string;
  modifiedAtMs?: number | null;
  pocReceipts: Array<{
    contractVersion: number;
    receiptId: string;
    scanId: string;
    candidateId: string;
    inventoryId: string;
    control: "positive" | "negative";
    language: string;
    scriptSha256: string;
    expectedObservationSha256: string;
    workspaceSha256: string;
    stdoutSha256: string;
    stderrSha256: string;
    expectedExitCode: number;
    exitCode?: number | null;
    passed: boolean;
    containment: string;
    artifactPath: string;
    execution?: {
      expectedObservation: string;
      startedAtMs: number;
      completedAtMs: number;
      timeoutMs: number;
      outputLimitBytes: number;
      sandboxProvider: string;
      sandboxProfileSha256: string;
      scriptPath: string;
      stdoutPath: string;
      stderrPath: string;
    } | null;
  }>;
  bundle: {
    scanId: string;
    mode: SecurityScanMode;
    model: string;
    scope: string;
    inventoryId: string;
    phase: string;
    coverage: Array<{ path: string; status: "reviewed" | "excluded"; reason?: string | null }>;
    supportingCoverage: Array<{
      path: string;
      status: "reviewed" | "excluded";
      reason?: string | null;
    }>;
    candidates: Array<{
      candidateId: string;
      ruleId: string;
      identityAnchor: string;
      identityInstance?: string | null;
      title: string;
      summary: string;
      category: string;
      cwe: string[];
      severity: SecuritySeverity;
      confidence: SecurityConfidence;
      impact: string;
      source: { path: string; line?: number | null; description: string };
      control: { path: string; line?: number | null; description: string };
      sink: { path: string; line?: number | null; description: string };
      remediation: string;
      validation: {
        disposition: "reportable" | "suppressed" | "not_applicable" | "deferred";
        evidence: string;
        counterevidence: string[];
      };
      poc: {
        goal: string;
        outcome:
          | "reproduced"
          | "partially_reproduced"
          | "not_reproduced"
          | "blocked"
          | "unsafe_to_execute";
        positiveReceiptId?: string | null;
        negativeReceiptId?: string | null;
        limitations: string[];
      };
      attackPath?: {
        attacker: string;
        entrypoint: string;
        preconditions: string[];
        path: string[];
        likelihood: string;
      } | null;
    }>;
  };
  seal?: {
    scanId: string;
    bundleDigest: string;
    reviewedFiles: number;
    excludedFiles: number;
    supportingFiles: number;
    deepPasses?: number | null;
    findings: Array<{
      findingId: string;
      candidateId: string;
      severity: SecuritySeverity;
      sourcePath: string;
      impact: string;
    }>;
  } | null;
}
