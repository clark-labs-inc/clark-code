// The single seam between the UI and `agent-core`.
//
// Surfaces only ever call this interface; they never know whether the engine is
// running native in the Tauri host (production) or as a mock (browser preview /
// tests). A future WASM build of agent-core slots in as a third implementation
// behind the same interface.

import type {
  ClientResponse,
  ProviderInfo,
  Session,
  Snapshot,
  ContentBlock,
  ResumeTranscript,
  MemoryOverview,
  CollaborationMode,
} from "./types";
import type { Upload } from "../lib/attachments";

export interface ConnectConfig {
  endpoint?: string;
  command?: string[];
  cwd?: string;
  auth_token?: string;
  /** Provider-specific extras (e.g. local coding: base_url, model, clark). */
  extra?: Record<string, unknown>;
  headers?: Record<string, string>;
}

export interface SessionOptions {
  cwd?: string;
  mode?: string;
  collaboration_mode?: CollaborationMode;
  /** Typed transcript replay for providers without server-side resume. */
  resume?: ResumeTranscript;
}

export interface CloudTrajectoryConfig {
  endpoint: string;
  token: string;
  title: string;
  provider: string;
  project?: string;
  repositoryFingerprint?: string;
  remoteHost?: string;
  mode?: string;
  metadata: Record<string, unknown>;
  /** Stable signed-in account scope used only to partition the native outbox. */
  ownerScope: string;
}

/** Read-only Git identity for the checkout backing the composer. */
export interface ProjectContext {
  branch: string;
  detached: boolean;
  isWorktree: boolean;
  worktreeRoot: string;
  activity: ProjectActivity;
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

export interface CoreBridge {
  listProviders(): Promise<ProviderInfo[]>;
  connect(providerId: string, config: ConnectConfig): Promise<void>;
  /** Re-run connect on a live session's EXISTING provider instance (keeps the
   *  session + transcript) — used to hot-swap model / reasoning effort
   *  mid-conversation. Native bridge only. */
  reconfigure?(sessionId: string, config: ConnectConfig): Promise<void>;
  /** Create a session on the just-connected provider. `bindId` — when reopening
   *  an existing conversation on a provider that can't resume — keys the new
   *  session (and its snapshot events) by that conversation id. */
  newSession(providerId: string, options: SessionOptions, bindId?: string): Promise<Session>;
  /** Resume a prior session by id (capability-gated: `load_session`). */
  loadSession(providerId: string, id: string): Promise<Session>;
  /** Drop a live session — destroys its provider and any running agent loop.
   *  Only called on archive/delete/sign-out; switching never closes. */
  closeSession?(sessionId: string): Promise<void>;
  /** Bind the native event stream to Clark's append-only trajectory store.
   * Native prompts are rejected until this succeeds, making the cloud the
   * durable source before local projection begins. */
  configureCloudTrajectory?(
    sessionId: string,
    config: CloudTrajectoryConfig,
    baseSnapshot: Snapshot,
    baseRev: number,
  ): Promise<void>;
  /** Replace the app-wide Clark cloud JWT after a sign-in refresh — in-flight
   *  trajectory retries read it per request (see `onCloudAuthExpired`). */
  updateCloudToken?(token: string): Promise<void>;
  /** The native trajectory sync hit a 401: refresh the sign-in and push the
   *  new token via `updateCloudToken`. Returns an unsubscribe fn. */
  onCloudAuthExpired?(handler: () => void): () => void;
  /** Best-effort cloud sync failed for part of a run (the run itself keeps
   *  going) — surface a non-blocking warning. Returns an unsubscribe fn. */
  onCloudSyncWarning?(handler: (message: string) => void): () => void;
  prompt(sessionId: string, blocks: ContentBlock[], attachments?: Upload[]): Promise<void>;
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
   * files) under `<cwd>/.clark/memory/`. Read-only. Native bridge only.
   */
  listMemory?(cwd: string, remote?: RemoteExecutorTarget | null): Promise<MemoryOverview>;
  /** List the user's global memory under `~/.clark/memory/`. Native bridge only. */
  listGlobalMemory?(): Promise<MemoryOverview>;
  /** Project-relative file paths under `cwd`, for the `@`-mention picker. */
  listFiles?(cwd: string, remote?: RemoteExecutorTarget | null): Promise<string[]>;
  /** Current branch and linked-worktree identity for the selected checkout. */
  projectContext?(
    cwd: string,
    remote?: RemoteExecutorTarget | null,
  ): Promise<ProjectContext | null>;
  /** Open a path in the OS default app, or reveal it in the file manager. */
  openPath?(path: string, reveal?: boolean): Promise<void>;
  /** Create a named, sibling Git worktree and return its absolute path. */
  createPermanentWorktree?(projectPath: string, name: string): Promise<string>;
  /** Inspect the platform sandbox without prompting or changing privilege. */
  localSandboxStatus?(cwd: string): Promise<LocalSandboxStatus>;
  /** Run the explicit, product-owned setup flow (UAC on Windows). */
  setupLocalSandbox?(cwd: string): Promise<LocalSandboxStatus>;
}

export interface RemoteExecutorTarget {
  ws_url: string;
  token: string;
}

let cached: CoreBridge | null = null;

/**
 * Returns the right bridge:
 * - the Tauri-backed bridge inside the desktop app;
 * - the DevBridge (real providers via the `devbridge` server) when the page is
 *   loaded with `?dev` — used for headless real-Clark testing and video;
 * - the mock otherwise (plain browser preview).
 */
export async function getBridge(): Promise<CoreBridge> {
  if (cached) return cached;
  const runningInTauri =
    typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  const params =
    typeof window !== "undefined" ? new URLSearchParams(window.location.search) : null;

  if (runningInTauri) {
    const { TauriBridge } = await import("./tauriBridge");
    cached = new TauriBridge();
  } else if (params?.has("dev")) {
    const { DevBridge } = await import("./devBridge");
    cached = new DevBridge(params.get("dev") || undefined);
  } else {
    const { MockBridge } = await import("./mockBridge");
    cached = new MockBridge();
  }
  return cached;
}
