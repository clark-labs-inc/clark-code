// The single seam between the UI and `agent-core`.
//
// Surfaces only ever call this interface; they never know whether the engine is
// running native in the Tauri host (production) or as a mock (browser preview /
// tests). A future WASM build of agent-core slots in as a third implementation
// behind the same interface.

import type {
  ClientResponse,
  ProviderInfo,
  SpecialistProjectionPublished,
  SpecialistCatalogAttestation,
  Session,
  Snapshot,
  ContentBlock,
  ResumeTranscript,
  MemoryOverview,
  SecurityScanRecord,
  CollaborationMode,
} from "./types";
import type { Upload } from "../lib/attachments";

export interface ConnectConfig {
  /** Only ACP may select a sidecar command; shipped providers are native-owned. */
  command?: string[];
  cwd?: string;
  /** Non-secret provider preferences and opaque native capability handles. */
  extra?: Record<string, unknown>;
}

export interface SessionOptions {
  cwd?: string;
  mode?: string;
  collaboration_mode?: CollaborationMode;
  /** Typed transcript replay for providers without server-side resume. */
  resume?: ResumeTranscript;
}

export type SessionOpenRequest =
  | { kind: "new"; options: SessionOptions; bindId?: string }
  | { kind: "load"; id: string };

export interface QuickChatWorkspace {
  id: string;
  path: string;
}

export interface ProjectDirectory {
  name: string;
  path: string;
}

/** Authoritative identity allocated by the provider for one submitted turn. */
export interface PromptReceipt {
  runId: string;
}

export interface CloudTrajectoryConfig {
  title: string;
  provider: string;
  project?: string;
  repositoryFingerprint?: string;
  remoteHost?: string;
  mode?: string;
  metadata: Record<string, unknown>;
}

/** Read-only Git identity for the checkout backing the composer. */
export interface ProjectContext {
  branch: string;
  detached: boolean;
  isWorktree: boolean;
  worktreeRoot: string;
  activity: ProjectActivity;
}

export interface ProjectBranch {
  name: string;
  /** Absolute checkout root when this branch is already attached to a worktree. */
  checkoutPath?: string | null;
}

export type ManagedWorktreeBase = "current" | "default";
export type WorktreeTransitionAction =
  | "create_isolated"
  | "open_owner"
  | "switch_clean"
  | "preserve_changes";
export type WorktreePreservation = "clean" | "changes_remain_in_source" | "owner_checkout";
export type ManagedWorktreeState = "ready" | "dirty" | "committed" | "saved" | "missing";

export interface WorktreeChangeSummary {
  changedFiles: number;
  untrackedFiles: number;
  conflictedFiles: number;
}

export interface ManagedWorktreeBaseOption {
  id: ManagedWorktreeBase;
  label: string;
  reference: string;
  revision: string;
  fallback: boolean;
}

/** Native, non-mutating decision record for a branch or isolation journey. */
export interface ProjectWorktreeTransitionPlan {
  sourceRoot: string;
  sourceBranch?: string | null;
  /** Null while the selected branch has not received its first commit. */
  sourceRevision: string | null;
  sourceChanges: WorktreeChangeSummary;
  sourceIsManaged: boolean;
  targetBranch?: string | null;
  targetCheckoutPath?: string | null;
  action: WorktreeTransitionAction;
  preservation: WorktreePreservation;
  requiresConfirmation: boolean;
  baseOptions: ManagedWorktreeBaseOption[];
  managedLocation: string;
}

export interface ManagedWorktreeRequest {
  base: ManagedWorktreeBase;
  label?: string | null;
  targetBranch?: string | null;
}

export interface ManagedWorktree {
  id: string;
  label: string;
  path: string;
  sourceRoot: string;
  base: ManagedWorktreeBase;
  baseReference: string;
  baseRevision: string;
  headRevision?: string | null;
  /** Local branch the agent created to protect detached commits before archival. */
  preservedBranch?: string | null;
  createdAtMs: number;
  state: ManagedWorktreeState;
  changes: WorktreeChangeSummary;
}

export interface ManagedWorktreeCleanupReceipt {
  id: string;
  path: string;
  removed: boolean;
}

export interface ManagedWorktreeBranchReceipt {
  id: string;
  path: string;
  branch: string;
  headRevision: string;
}

export interface ProjectActivity {
  changedFiles: number;
  untrackedFiles: number;
  conflictedFiles: number;
  externalAgents: ExternalAgentActivity[];
  detectedAtMs: number;
}

export interface ExternalAgentActivity {
  id: string;
  title: string;
  agentNickname?: string | null;
  updatedAtMs: number;
}

export type LocalSandboxState = "enforced" | "setup_required" | "unavailable";

export interface LocalSandboxStatus {
  state: LocalSandboxState;
  backend: "macos_seatbelt" | "linux_bubblewrap" | "windows_restricted_token";
  reason?: string | null;
  setup_available: boolean;
}

export interface ComputerUsePermissionStatus {
  accessibility: boolean;
  screen_recording: boolean;
  screen_recording_restart_required: boolean;
}

export interface ComputerUsePlatformStatus {
  supported: boolean;
  platform: string;
  service_ready: boolean;
  readiness: "unsupported" | "service_unavailable" | "needs_permission" | "restart_required" | "ready";
  permission_owner?: {
    display_name: string;
    bundle_id: string;
  } | null;
  permissions?: ComputerUsePermissionStatus | null;
  detail?: string | null;
}

export interface ComputerUseAppApproval {
  identity_key: string;
  bundle_id: string;
  app_name: string;
  team_identifier?: string | null;
  granted_at_ms: number;
  last_used_at_ms: number;
}

export interface ComputerUseApprovalSnapshot {
  revision: number;
  approvals: ComputerUseAppApproval[];
}

export type ComputerUseActionKind =
  | "click"
  | "type_text"
  | "keypress"
  | "scroll"
  | "drag"
  | "secondary_action"
  | "select_text"
  | "set_value";

export interface ComputerUseActionReceipt {
  receipt_id: string;
  prepared_action_id: string;
  application_identity_key: string;
  bundle_id: string;
  pid: number;
  window_id: number;
  action_kind: ComputerUseActionKind;
  disposition:
    | "deny"
    | "mandatory_handoff"
    | "action_time_confirmation"
    | "preapproval_eligible"
    | "allow";
  outcome: "succeeded" | "dry_run" | "cancelled" | "user_takeover" | "failed";
  payload_summary: string;
  completed_at_ms: number;
  persisted: boolean;
}

export interface SkillCatalogEntry {
  id: string;
  revision: string;
  name: string;
  invocationName: string;
  description: string;
  scope: "bundled" | "project" | "user";
  origin: "bundled" | "compatible" | "claude" | "plugin";
  source: string;
  requiredTools: string[];
  missingTools: string[];
  allowImplicitInvocation: boolean;
  enabled: boolean;
  disabledReason?: string | null;
  hasNameCollision: boolean;
}

export interface SkillDiagnostic {
  severity: "warning" | "error";
  code: string;
  message: string;
  source?: string | null;
}

export interface SkillCatalogSnapshot {
  revision: string;
  environmentId: string;
  projectRoot: string;
  skills: SkillCatalogEntry[];
  diagnostics: SkillDiagnostic[];
}

export interface SkillCatalogChange {
  changed: boolean;
  revision: string;
  snapshot?: SkillCatalogSnapshot | null;
}

export interface InstructionProvenance {
  path: string;
  scope: "personal" | "project" | "nested";
  origin: "agent_home" | "compatible" | "claude";
  precedence: number;
  bytesLoaded: number;
  truncated: boolean;
}

export interface ProjectInstructions {
  text: string;
  sources: InstructionProvenance[];
}

export type SkillPackScope = "project" | "user";
export type SkillPackAction = "installed" | "updated" | "unchanged" | "uninstalled";

export interface InstalledSkillPack {
  packId: string;
  revision: string;
  source: string;
  skillCount: number;
  scope: SkillPackScope;
  installRoot: string;
}

export interface SkillPackReceipt {
  action: SkillPackAction;
  packId: string;
  revision?: string | null;
  previousRevision?: string | null;
  skillCount: number;
  scope: SkillPackScope;
  installRoot: string;
  warnings: string[];
}

export interface SkillPackOperationResult {
  receipt: SkillPackReceipt;
  catalog: SkillCatalogSnapshot;
}

export interface CoreBridge {
  listProviders(): Promise<ProviderInfo[]>;
  /** Versioned product specialist manifests embedded in and protected by
   * the native app bundle. */
  listSpecialistCatalog?(): Promise<SpecialistCatalogAttestation>;
  /** Allocate or reopen an app-managed checkout for a repository-free chat. */
  prepareQuickChatWorkspace?(id?: string): Promise<QuickChatWorkspace>;
  /** Re-run connect on a live session's EXISTING provider instance (keeps the
   *  session + transcript) — used to hot-swap model / reasoning effort
   *  mid-conversation. Native bridge only. */
  reconfigure?(sessionId: string, config: ConnectConfig): Promise<void>;
  /** Add explicit read-only filesystem roots without replacing the live
   * conversation, transcript, or writable document workspace. */
  addReadRoots?(sessionId: string, roots: string[]): Promise<void>;
  /** Revoke explicit read-only filesystem roots from a live conversation. */
  removeReadRoots?(sessionId: string, roots: string[]): Promise<void>;
  /** Atomically construct, connect, and bind one provider/session. `bindId`
   * keys a non-resumable provider by the durable conversation id. */
  openSession(
    providerId: string,
    config: ConnectConfig,
    request: SessionOpenRequest,
  ): Promise<Session>;
  /** Drop a live session — destroys its provider and any running agent loop.
   *  Only called on archive/delete/sign-out; switching never closes. */
  closeSession?(sessionId: string): Promise<void>;
  /** Bind the native event stream to the agent's append-only trajectory store.
   * Native prompts are rejected until this succeeds, making the cloud the
   * durable source before local projection begins. */
  configureCloudTrajectory?(
    sessionId: string,
    config: CloudTrajectoryConfig,
    baseSnapshot: Snapshot,
    baseRev: number,
  ): Promise<void>;
  /** The native trajectory sync hit a 401. Returns an unsubscribe fn; refreshing
   *  through native Google exchange rotates the host-owned credential. */
  onCloudAuthExpired?(handler: () => void): () => void;
  /** Clear the native the agent credential and account binding during sign-out. */
  /** Best-effort cloud sync failed for part of a run (the run itself keeps
   *  going) — surface a non-blocking warning. Returns an unsubscribe fn. */
  onCloudSyncWarning?(handler: (message: string) => void): () => void;
  /** A native Scientist/RSI projection was durably accepted by the agent
   * cloud and its canvas can refresh from the authoritative endpoint. */
  onSpecialistProjectionPublished?(
    handler: (receipt: SpecialistProjectionPublished) => void,
  ): () => void;
  /** Another device deleted a live conversation. The desktop must stop its
   * local session rather than recreate that cloud history. */
  onCloudConversationDeleted?(handler: (conversationId: string) => void): () => void;
  prompt(
    sessionId: string,
    blocks: ContentBlock[],
    attachments?: Upload[],
  ): Promise<PromptReceipt>;
  /** Resume an active standing goal from its latest durable recovery point
   * without manufacturing a visible user message. Native bridge only. */
  resumeSavedProgress?(sessionId: string): Promise<PromptReceipt>;
  /** Replace the provider's model-visible history with a compact summary.
   *  This is a standalone control operation, not a user prompt. */
  compact?(sessionId: string): Promise<void>;
  /** Inject a user message into the session's ACTIVE run (mid-run steering).
   *  Rejects when the provider has no live run or no steering support —
   *  callers fall back to queueing the message as a normal follow-up. */
  steer?(sessionId: string, blocks: ContentBlock[]): Promise<void>;
  cancel(sessionId: string, runId: string): Promise<void>;
  respond(sessionId: string, response: ClientResponse): Promise<void>;
  /** Best-effort: ask the provider to switch the session's named mode (e.g.
   *  "plan"). Not every bridge/provider supports this — callers should treat
   *  a rejected promise as a silent no-op. */
  setMode?(sessionId: string, mode: string): Promise<void>;
  /** Switch read-only planning independently of provider-native modes and approvals. */
  setCollaborationMode?(sessionId: string, mode: CollaborationMode): Promise<void>;
  /** Best-effort: switch the session's output style (see `lib/outputStyle.ts`). */
  setOutputStyle?(sessionId: string, style: string): Promise<void>;
  /** `/btw` — answer a one-off side question against the session's current
   *  context WITHOUT interrupting the active run. Returns the answer text for
   *  an overlay to render; never cancels the main run. */
  sideQuestion?(sessionId: string, question: string): Promise<string>;
  /** Subscribe to snapshot updates for ALL live sessions. Each snapshot is
   *  tagged with its session id (`snapshot.session`); the handler routes it.
   *  Returns an unsubscribe fn. */
  subscribe(handler: (snapshot: Snapshot) => void): () => void;
  /**
   * List the project-scoped memory (the `MEMORY.md` index plus any per-fact
   * files) for a live conversation's native-bound checkout. Read-only.
   */
  listMemory?(sessionId: string): Promise<MemoryOverview>;
  /** List global memory from the authenticated account's native partition. */
  listGlobalMemory?(): Promise<MemoryOverview>;
  /** Project-relative file paths under `cwd`, for the `@`-mention picker. */
  listFiles?(cwd: string, remote?: RemoteWorkerTarget | null): Promise<string[]>;
  /** Immediate folders beside `cwd`, for repository-aware `@` autocomplete. */
  listSiblingDirectories?(
    cwd: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<ProjectDirectory[]>;
  /** Canonical Security scanner bundles and seals under the selected checkout. */
  listSecurityScans?(
    cwd: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<SecurityScanRecord[]>;
  /** Current canonical skill catalog for this local or remote environment. */
  listSkills?(
    cwd: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<SkillCatalogSnapshot>;
  /** Force discovery and publish a change event when the revision differs. */
  reloadSkills?(
    cwd: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<SkillCatalogSnapshot>;
  /** Poll-friendly delta check used while the composer is active. */
  skillChanges?(
    cwd: string,
    sinceRevision: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<SkillCatalogChange>;
  onSkillsChanged?(handler: (snapshot: SkillCatalogSnapshot) => void): () => void;
  listInstructions?(
    cwd: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<ProjectInstructions | null>;
  listSkillPacks?(
    cwd: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<InstalledSkillPack[]>;
  installSkillPack?(
    cwd: string,
    request: { packId: string; sourcePath: string; scope: SkillPackScope },
    remote?: RemoteWorkerTarget | null,
  ): Promise<SkillPackOperationResult>;
  uninstallSkillPack?(
    cwd: string,
    packId: string,
    scope: SkillPackScope,
    remote?: RemoteWorkerTarget | null,
  ): Promise<SkillPackOperationResult>;
  /** Current branch and linked-worktree identity for the selected checkout. */
  projectContext?(
    cwd: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<ProjectContext | null>;
  /** Open a path in the OS default app, or reveal it in the file manager. */
  openPath?(path: string, reveal?: boolean): Promise<void>;
  /** Existing local branches and the checkout that currently owns each one. */
  listProjectBranches?(
    projectPath: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<ProjectBranch[]>;
  /** Switch a clean selected checkout to an existing local branch. */
  switchProjectBranch?(
    projectPath: string,
    branch: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<void>;
  /** Create a named sibling worktree from the latest advertised origin/main. */
  createPermanentWorktree?(
    projectPath: string,
    name: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<string>;
  /** Plan a branch/worktree move without touching Git state. */
  planProjectWorktree?(
    projectPath: string,
    targetBranch?: string | null,
  ): Promise<ProjectWorktreeTransitionPlan>;
  /** Create a branch-backed the agent-managed worktree from the selected base. */
  createManagedWorktree?(
    projectPath: string,
    request: ManagedWorktreeRequest,
  ): Promise<ManagedWorktree>;
  /** List only the agent-managed worktrees for a repository. */
  listManagedWorktrees?(projectPath: string): Promise<ManagedWorktree[]>;
  /** Explicitly remove one clean the agent-managed worktree. */
  cleanupManagedWorktree?(
    projectPath: string,
    id: string,
  ): Promise<ManagedWorktreeCleanupReceipt>;
  /** Save a detached managed checkout's commits under a durable local branch. */
  saveManagedWorktreeBranch?(
    projectPath: string,
    id: string,
  ): Promise<ManagedWorktreeBranchReceipt>;
  /** Inspect the platform sandbox without prompting or changing privilege. */
  localSandboxStatus?(cwd: string): Promise<LocalSandboxStatus>;
  /** Run the explicit, product-owned setup flow (UAC on Windows). */
  setupLocalSandbox?(cwd: string): Promise<LocalSandboxStatus>;
  /** Native computer-use readiness and OS permission preflight. */
  computerUsePlatformStatus?(): Promise<ComputerUsePlatformStatus>;
  /** Explicitly request the OS-level Accessibility and Screen Recording grants. */
  requestComputerUsePermissions?(): Promise<ComputerUsePermissionStatus>;
  /** Signer-bound per-app approvals currently stored by the native helper. */
  computerUseApprovalSnapshot?(): Promise<ComputerUseApprovalSnapshot>;
  /** Revoke one durable app grant and return the resulting snapshot. */
  revokeComputerUseApproval?(identityKey: string): Promise<ComputerUseApprovalSnapshot>;
  /** Revoke all durable app grants and return the resulting snapshot. */
  revokeAllComputerUseApprovals?(): Promise<ComputerUseApprovalSnapshot>;
  /** Redacted, bounded native action receipts, newest receipt last. */
  recentComputerUseReceipts?(): Promise<ComputerUseActionReceipt[]>;
}

export interface RemoteWorkerTarget {
  /** Opaque handle resolved only by the native remote runtime registry. */
  id: string;
}

let cached: CoreBridge | null = null;
let loading: Promise<CoreBridge> | null = null;

/**
 * Returns the right bridge:
 * - the Tauri-backed bridge inside the desktop app;
 * - the DevBridge (real providers via the `devbridge` server) when the page is
 *   loaded with `?dev` — used for headless real-the agent testing and video;
 * - the mock otherwise (plain browser preview).
 */
export async function getBridge(): Promise<CoreBridge> {
  if (cached) return cached;
  if (!loading) {
    loading = (async () => {
      const runningInTauri =
        typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
      const params =
        typeof window !== "undefined" ? new URLSearchParams(window.location.search) : null;

      // Deterministic native-WebView acceptance runs need the real Tauri shell
      // without spending hosted-model tokens. Vite leaves this disabled unless
      // an explicit acceptance build opts in at compile time.
      if (import.meta.env.VITE_FORCE_MOCK_BRIDGE === "1") {
        const { MockBridge } = await import("./mockBridge");
        return new MockBridge();
      }
      if (runningInTauri) {
        const { TauriBridge } = await import("./tauriBridge");
        return new TauriBridge();
      }
      if (params?.has("dev")) {
        const { DevBridge } = await import("./devBridge");
        return new DevBridge(params.get("dev") || undefined);
      }
      const { MockBridge } = await import("./mockBridge");
      return new MockBridge();
    })();
  }
  try {
    cached = await loading;
    return cached;
  } finally {
    loading = null;
  }
}
