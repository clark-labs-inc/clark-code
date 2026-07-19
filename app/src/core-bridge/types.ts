// Hand-mirrored from the `agent-core` Rust types; keep them in sync with
// crates/agent-core/src/{domain,projection,provider}.rs.

export type Role = "user" | "agent" | "system";
export type MessagePhase = "commentary" | "final_answer";

export type ContentBlock =
  | { type: "text"; text: string }
  | { type: "thinking"; text: string }
  | { type: "image"; mime_type: string; data: string; uri?: string }
  | { type: "audio"; mime_type: string; data: string }
  | { type: "resource"; uri: string; mime_type?: string; text?: string }
  | { type: "resource_link"; uri: string; name?: string };

export type ToolKind =
  | "read" | "edit" | "delete" | "move"
  | "search" | "execute" | "think" | "fetch" | "research"
  | "view_image" | "generate_image" | "other";

export type ToolStatus = "pending" | "in_progress" | "completed" | "failed";

export interface FsLocation {
  path: string;
  line?: number;
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
    };

export interface ResumeTranscript {
  items: ResumeItem[];
  truncated: boolean;
}

export type PlanPhaseStatus = "pending" | "in_progress" | "completed";

export interface PlanPhase {
  title: string;
  status: PlanPhaseStatus;
  priority?: string;
}

export interface Plan {
  phases: PlanPhase[];
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
  | "provider_error"
  | "rate_limited"
  | "transport_error"
  | "context_overflow"
  | "insufficient_credits"
  | "tool_fatal"
  | "local_state"
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
  outcome?: RunOutcome;
  /** Pre-run working-tree checkpoint used as a change-tracking baseline. */
  checkpoint?: string;
}

export type TimelineItem =
  | { item: "message"; run: string; role: Role; blocks: ContentBlock[]; phase?: MessagePhase }
  | { item: "tool_call"; id: string }
  | { item: "artifact"; id: string }
  | { item: "plan"; run?: string; plan?: Plan };

export interface Snapshot {
  session?: string;
  runs: Record<string, RunView>;
  timeline: TimelineItem[];
  tool_calls: Record<string, ToolCall>;
  plan?: Plan;
  pending_permission?: PermissionRequest;
  artifacts: Artifact[];
  focus?: WorkspaceFocus;
  fan_out?: FanOut;
}

export function emptySnapshot(): Snapshot {
  return { runs: {}, timeline: [], tool_calls: {}, artifacts: [] };
}

export interface ProviderCapabilities {
  streaming: boolean;
  permissions: boolean;
  fs: boolean;
  terminal: boolean;
  load_session: boolean;
  modes: string[];
}

export interface ProviderInfo {
  id: string;
  label: string;
  capabilities: ProviderCapabilities;
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
  environment?: SessionEnvironment;
}

export type ClientResponse = {
  kind: "permission";
  request: string;
  option: string;
  /** Optional free-text attached to the choice — e.g. plan feedback on a
   *  "keep planning" rejection, delivered to the model as the rejection
   *  reason. */
  feedback?: string;
};

/** One per-fact memory file under `<cwd>/.clark/memory/`. */
export interface MemoryFactView {
  file: string;
  name?: string | null;
  description?: string | null;
  kind?: string | null;
  body: string;
}

/** The per-repository memory for one project folder (index + fact files). */
export interface MemoryOverview {
  /** Absolute path to `<cwd>/.clark/memory`. */
  dir: string;
  /** Whether a `MEMORY.md` index has been written. */
  exists: boolean;
  /** Contents of the always-loaded `MEMORY.md` index, if present. */
  index?: string | null;
  /** Per-fact memory files (newest first). */
  facts: MemoryFactView[];
}
