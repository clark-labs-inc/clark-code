// Hand-mirrored from the `agent-core` Rust types; keep them in sync with
// crates/agent-core/src/{domain,projection,provider}.rs.

export type Role = "user" | "agent" | "system";

export type ContentBlock =
  | { type: "text"; text: string }
  | { type: "image"; mime_type: string; data: string; uri?: string }
  | { type: "audio"; mime_type: string; data: string }
  | { type: "resource"; uri: string; mime_type?: string; text?: string }
  | { type: "resource_link"; uri: string; name?: string };

export type ToolKind =
  | "read" | "edit" | "delete" | "move"
  | "search" | "execute" | "think" | "fetch" | "other";

export type ToolStatus = "pending" | "in_progress" | "completed" | "failed";

export interface FsLocation {
  path: string;
  line?: number;
}

export interface ToolCall {
  id: string;
  title: string;
  kind: ToolKind;
  status: ToolStatus;
  locations: FsLocation[];
  content: ContentBlock[];
  raw_input?: unknown;
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
}

export type WorkspaceSurfaceKind = "browser" | "terminal" | "files" | "website";

export interface WorkspaceFocus {
  surface: WorkspaceSurfaceKind;
  path?: string;
  url?: string;
  is_dir?: boolean;
  tool_call?: string;
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

export interface RunOutcome {
  status: RunStatus;
  stop_reason?: string;
  error?: string;
}

export interface RunView {
  id: string;
  status: RunStatus;
  outcome?: RunOutcome;
}

export type TimelineItem =
  | { item: "message"; run: string; role: Role; blocks: ContentBlock[] }
  | { item: "tool_call"; id: string }
  | { item: "artifact"; id: string }
  | { item: "plan" };

export interface Snapshot {
  session?: string;
  runs: Record<string, RunView>;
  timeline: TimelineItem[];
  tool_calls: Record<string, ToolCall>;
  plan?: Plan;
  pending_permission?: PermissionRequest;
  artifacts: Artifact[];
  focus?: WorkspaceFocus;
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

export interface Session {
  id: string;
  provider: string;
  capabilities: ProviderCapabilities;
  mode?: string;
}

export type ClientResponse = {
  kind: "permission";
  request: string;
  option: string;
};
