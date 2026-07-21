import { create } from "zustand";
import {
  getBridge,
  type CloudTrajectoryConfig,
  type CoreBridge,
  type SessionOptions,
} from "../core-bridge/bridge";
import { syncFanOut, resetFanOut } from "./fanOutStore";
import {
  emptySnapshot,
  type ClientResponse,
  type CollaborationMode,
  type PlanImplementationContext,
  type ProviderInfo,
  type Session,
  type Snapshot,
  type MemoryOverview,
} from "../core-bridge/types";
import {
  fileToAttachment,
  toUpload,
  MAX_ATTACHMENT_BYTES,
  type PendingAttachment,
  type Upload,
} from "../lib/attachments";
import { minLoadDuration } from "../lib/minLoadDuration";
import {
  loadAuthSession,
  refreshAuthSession,
  signInWithGoogle,
  signOut as authSignOut,
  type AuthMethod,
  type AuthSession,
} from "../lib/auth";
import {
  drainLocalHistory,
  buildResumeTranscript,
  snapshotBeforeTimelineItem,
  settleRuns,
  deriveTitle,
  hasContent,
  type ConversationMeta,
} from "../lib/history";
import {
  loadLocalSettings,
  saveLocalSettings,
  localConnectConfig,
  loadRecentProjects,
  addRecentProject,
  loadMemoriesEnabled,
  saveMemoriesEnabled,
  loadBrowserEnabled,
  saveBrowserEnabled,
  loadOrchestrationEnabled,
  saveOrchestrationEnabled,
  localSettingsReady,
  loadChatModels,
  saveChatModels,
  effectiveModelSettings,
  normalizeReasoningEffort,
  type LocalAgentSettings,
  type ChatModelOverride,
} from "../lib/localAgent";
import { pickFolder } from "../lib/pickFolder";
import { sshConnect, sshDisconnect, remoteTarget, type RemoteInfo } from "../lib/ssh";
import { loadSshHosts, hostReady, type SshHost } from "../lib/sshHosts";
import {
  loadApprovalPolicy,
  loadCollaborationMode,
  saveApprovalPolicy,
  saveCollaborationMode,
  pickAllowOption,
  wouldAutoApprove,
  nextApprovalPolicy,
  type ApprovalPolicy,
} from "../lib/permissions";
import { loadOutputStyle, saveOutputStyle } from "../lib/outputStyle";
import {
  cloudCreds,
  cloudList,
  cloudGet,
  cloudDelete,
  cloudSetArchived,
  cloudShare,
  cloudUnshare,
  scheduleCloudPut,
  flushCloudPuts,
} from "../lib/cloudHistory";
import {
  provisionCodeKey,
  billingMe,
  latestActivityReward,
  type ActivityReward,
  type BillingSummary,
} from "../lib/account";
import { copyText } from "../lib/clipboard";
import { notify } from "../lib/notify";
import { repositoryFingerprintForRoot } from "../lib/repositoryKnowledge";
import { conversationProjectRoot, liveProjectRoot } from "../lib/sessionEnvironment";
import {
  checkAndStageUpdate,
  installStagedUpdate,
  beginUpdateDrain,
  cancelUpdateDrain,
  relaunchApp,
  consumeJustUpdated,
  type StagedUpdate,
  type DownloadProgress,
  type UpdateCheckResult,
} from "../lib/updater";
import { onSettingsMenuRequested, onUpdateMenuRequested } from "../lib/nativeMenu";
import { updateDrainBlockerCount } from "../lib/updateDrain";

/** A follow-up message the user sent while a run was active. It sends
 *  automatically when the run finishes — Codex-style, never interrupting. */
/** The sections of the unified Settings view (left-rail order). */
export type SettingsSection =
  | "general"
  | "project"
  | "integrations"
  | "commands"
  | "account"
  | "about";

export interface QueuedMessage {
  id: string;
  text: string;
  uploads: Upload[];
}

export interface ComposerPrefill {
  text: string;
  /** Present only for edit-and-resend; identifies the turn to replace. */
  timelineIndex?: number;
}

/** `/btw` overlay state. `loading` is true while the forked side-question call
 *  is in flight; a stale token (bumped on dismiss) lets a late result be
 *  dropped without clobbering a closed/newer overlay. */
export interface SideQuestionState {
  question: string;
  answer: string | null;
  error: string | null;
  loading: boolean;
  /** Monotonic token; only a result that matches this token is applied. */
  token: number;
}

function isBusy(snap: Snapshot): boolean {
  return Object.values(snap.runs).some(
    (r) => r.status === "running" || r.status === "queued",
  );
}

/** Past transcript (prefix) + live resumed turns → one displayed snapshot. */
function mergeHistory(prefix: Snapshot, live: Snapshot): Snapshot {
  const artifacts = [...prefix.artifacts];
  const idx = new Map(artifacts.map((a, i) => [a.id, i]));
  for (const a of live.artifacts) {
    const at = idx.get(a.id);
    if (at != null) artifacts[at] = a;
    else {
      idx.set(a.id, artifacts.length);
      artifacts.push(a);
    }
  }
  return {
    session: live.session ?? prefix.session,
    runs: { ...prefix.runs, ...live.runs },
    timeline: [...prefix.timeline, ...live.timeline],
    tool_calls: { ...prefix.tool_calls, ...live.tool_calls },
    execution_checklist: live.execution_checklist ?? prefix.execution_checklist,
    proposed_plan: live.proposed_plan ?? prefix.proposed_plan,
    goal: live.goal ?? prefix.goal,
    pending_permission: live.pending_permission,
    artifacts,
    focus: live.focus ?? prefix.focus,
    // Without this the fan-out is stripped from every reopened conversation
    // (its snapshot always has a history prefix), so the swarm panel never
    // renders — it only worked for brand-new sessions.
    fan_out: live.fan_out ?? prefix.fan_out,
  };
}

interface SessionState {
  bridge: CoreBridge | null;
  providers: ProviderInfo[];
  activeProvider: string | null;
  session: Session | null;
  snapshot: Snapshot;
  connecting: boolean;
  error: string | null;
  /** Transient success/info toast (e.g. "Share link copied"). Auto-dismisses. */
  notice: string | null;
  /** Transient non-fatal warning toast (e.g. cloud sync hiccup mid-run). */
  warning: string | null;
  /** Run ids whose failed/stopped terminal banner was dismissed this session. */
  dismissedFailedRuns: string[];
  /** Authenticated user + the Clark connection config it carries. */
  auth: AuthSession | null;
  /** Files staged to send with the next message. */
  attachments: PendingAttachment[];
  /** Saved conversations, newest first — the cloud is the source of truth. */
  conversations: ConversationMeta[];
  /** True while the first cloud conversation-list fetch is in flight. */
  conversationsLoading: boolean;
  /** Restored transcript when a past conversation is reopened (prefix to live). */
  historyPrefix: Snapshot | null;
  /** Conversation ids whose live session currently has a running run — drives
   *  the per-row "Working…" indicator in the sidebar. Any number of sessions
   *  can stream at once; switching between them never cancels a run. */
  runningIds: string[];
  /** Conversation ids selected in the sidebar (Shift-click). Drives the
   *  right-click bulk actions (archive / delete all selected). A fresh Set
   *  on every mutation so zustand re-renders. */
  selectedConversationIds: Set<string>;
  /** A session connect is in flight — drives the "Opening…" loading screen (and
   *  the sidebar row spinner) so the UI never looks frozen: remote connects
   *  bring up an SSH tunnel, which can take 10–20s. `kind` picks the copy
   *  ("Connecting" for a new session, "Reconnecting"/"Opening" for a reopen).
   *  Cleared when the session is live, the connect fails, or the user cancels
   *  (endSession). */
  opening: {
    id: string | null;
    kind: "start" | "open";
    title: string;
    remoteHost: string | null;
  } | null;
  /** Text staged into the composer, optionally tied to a sent turn to replace. */
  composerPrefill: ComposerPrefill | null;
  /** Config for the "Local coding" provider (persisted to localStorage). */
  localSettings: LocalAgentSettings;
  /** Per-conversation model + reasoning-effort settings, keyed by conversation
   *  id and pinned when the chat is created or first reopened. Legacy chats
   *  fall back to `localSettings` only until that first open. Persisted to
   *  localStorage (the cloud stores transcripts, not model prefs). */
  chatModels: Record<string, ChatModelOverride>;
  /** Where the next session runs: this machine, or a remote host over SSH. */
  projectMode: "local" | "remote";
  /** The saved SSH host selected for a remote session (id into sshHosts). */
  selectedHostId: string | null;
  /** The live remote connection for the active session (null when local). Held
   *  for teardown (ssh_disconnect) and to tag the conversation as remote. */
  activeRemote: RemoteInfo | null;
  /** The SSH destination of the active remote session, for the history badge. */
  activeRemoteHost: string | null;
  /** Authoritative project root for the active conversation. Unlike
   * `localSettings.cwd`, this does not change when the new-session picker does. */
  activeProjectRoot: string | null;
  /** Whether durable memory is enabled (global user preference; the agent gets
   *  the `memory` tool and its saved facts are injected into the prompt). */
  memoriesEnabled: boolean;
  /** Whether the experimental `browser` tool is enabled (off by default —
   *  downloads clark-browser, ~150-300MB, on first use). */
  browserEnabled: boolean;
  /** Whether bounded parallel repository work is available. The model-facing
   *  policy still requires an explicit trigger; writers run in safe copies. */
  orchestrationEnabled: boolean;
  /** Last memory status message (e.g. a load error). */
  memoryStatus: string | null;
  /** Whether the memory viewer popover is open. */
  memoryViewerOpen: boolean;
  /** True while the memory viewer is (re)loading. */
  loadingMemory: boolean;
  /** The last-loaded project-scope memory for the active folder. */
  memoryOverview: MemoryOverview | null;
  /** The last-loaded global-scope (per-user) memory. */
  globalMemoryOverview: MemoryOverview | null;
  /** Recently opened project folders (newest first). */
  recentProjects: string[];
  /** Follow-up messages sent while a run is active; drained when it finishes. */
  queued: QueuedMessage[];
  /** How agent permission requests are approved (Codex-style). */
  approvalPolicy: ApprovalPolicy;
  /** Read-only planning is independent from action approval policy. */
  collaborationMode: CollaborationMode;
  /** The agent's reply tone/persona for this session — see `lib/outputStyle.ts`. */
  outputStyle: string;
  /** Whether the in-chat terminal drawer is open. */
  terminalOpen: boolean;
  /** One-shot request to open a fresh terminal tab rooted at `cwd`. `nonce`
   *  increments per request so the panel only reacts to the latest one. */
  terminalLaunch: { cwd: string; nonce: number } | null;
  /** Whether the MCP servers settings modal is open. */
  mcpOpen: boolean;
  /** Whether the remote-hosts (SSH) settings modal is open. */
  sshOpen: boolean;
  /** Whether the unified Settings modal is open, and which section it shows. */
  settingsOpen: boolean;
  settingsSection: SettingsSection;
  /** Whether the ⌘K command palette is open. */
  paletteOpen: boolean;
  /** `/btw` side-question overlay state. A forked, tool-less model call over
   *  the session context that never interrupts the active run. Null when the
   *  overlay is closed. */
  sideQuestion: SideQuestionState | null;
  /** Whether the sidebar is collapsed to its icon rail. */
  sidebarCollapsed: boolean;
  /** Billing summary (plan, subscription, credits) from Clark; null until loaded. */
  billing: BillingSummary | null;
  loadingBilling: boolean;
  /** A fresh server-issued reward earned by completed paid activity. */
  activityReward: ActivityReward | null;
  /** A downloaded + staged app update awaiting a relaunch to apply. */
  update: StagedUpdate | null;
  /** Live byte progress while an update downloads in the background; null when idle. */
  updateProgress: DownloadProgress | null;
  /** A manifest check is in flight, including the gap before download progress starts. */
  updateChecking: boolean;
  /** True from "Restart to update" being clicked until the relaunch takes. */
  updateApplying: boolean;
  /** An update was requested and is waiting for active/queued work to settle. */
  updateWaiting: boolean;
  /** Set once on the first launch after an update applied — the version we're now on. */
  justUpdatedTo: string | null;

  init: () => Promise<void>;
  loadBilling: () => Promise<void>;
  selectProvider: (id: string) => void;
  setLocalSettings: (patch: Partial<LocalAgentSettings>) => void;
  setProjectMode: (mode: "local" | "remote") => void;
  setSelectedHostId: (id: string | null) => void;
  setProjectFolder: (path: string) => void;
  pickProjectFolder: () => Promise<void>;
  setMemoriesEnabled: (on: boolean) => void;
  setBrowserEnabled: (on: boolean) => void;
  setOrchestrationEnabled: (on: boolean) => void;
  loadMemory: () => Promise<void>;
  toggleMemoryViewer: () => void;
  setMemoryViewerOpen: (open: boolean) => void;
  signIn: (method: AuthMethod) => Promise<void>;
  signOutAuth: () => void;
  /** Mint + store a Clark Code API key for the signed-in user if none yet. */
  ensureCodeKey: () => Promise<void>;
  /** Fetch the account's conversation list from the cloud (the source of truth). */
  syncCloudIndex: () => Promise<void>;
  /** One-time: lift any chats left in localStorage by prior local-first versions
   *  into the cloud, then forget them locally. No-op once drained. */
  migrateLocalToCloud: () => void;
  /** Why the active environment can't start a session yet (folder unset, remote
   *  host not ready…), or null when ready. Lets the composer gate a pre-session
   *  submit with the same logic the start screen uses. */
  startBlockedReason: () => string | null;
  startSession: () => Promise<void>;
  /** Detach from the current conversation (→ welcome screen). Its live session
   *  keeps running in the background pool — reopening reattaches instantly.
   *  `force` (sign-out) tears down every live session instead. */
  endSession: (opts?: { force?: boolean }) => void;
  openConversation: (id: string) => Promise<void>;
  /** Soft-delete: hide from the main list but keep the transcript locally and in
   *  the cloud so it can be restored. Clears the view if it's the open chat. */
  archiveConversation: (id: string) => void;
  /** Bring an archived conversation back into the active list. */
  restoreConversation: (id: string) => void;
  /** Permanently delete a conversation — local cache AND the cloud copy. */
  deleteConversation: (id: string) => void;
  /** Rename a conversation; the manual title stops auto-derivation clobbering it. */
  renameConversation: (id: string, title: string) => void;
  /** Toggle one conversation in the sidebar's Shift-click selection. */
  toggleConversationSelection: (id: string) => void;
  /** Set the sidebar selection (replace). Pass an empty Set to clear. */
  setConversationSelection: (ids: Set<string>) => void;
  /** Archive every selected conversation at once. Busy ones are skipped (a
   *  notice is flashed); the rest are closed + flagged archived in the cloud. */
  archiveSelectedConversations: () => void;
  /** Permanently delete every selected conversation at once. Busy ones are
   *  skipped (a notice is flashed); the rest are closed + deleted from the
   *  cloud. Selection is cleared afterwards. */
  deleteSelectedConversations: () => void;
  /** Change the coding model / reasoning effort for the ACTIVE conversation.
   *  Writes a per-chat override (so other chats keep their own model), persists
   *  it, and — when a local session is live — hot-swaps that session's provider
   *  LLM (the transcript is kept; the next turn continues with full context on
   *  the new model). With no active chat, edits the global default instead. */
  updateModelSettings: (patch: { model?: string; reasoningEffort?: string }) => Promise<void>;
  /** Stage text in the composer. A timeline index makes submit replace that
   * turn and its abandoned suffix instead of appending a duplicate. */
  setComposerPrefill: (text: string | null, timelineIndex?: number) => void;
  /** Create a public read-only link for the viewed conversation + copy it. */
  shareConversation: () => Promise<void>;
  /** Revoke the viewed conversation's public link. */
  unshareConversation: () => Promise<void>;
  addFiles: (files: File[]) => Promise<void>;
  removeAttachment: (id: string) => void;
  send: (text: string) => Promise<void>;
  /** Replace one prior Clark Code user turn and rerun from the retained prefix. */
  resendFrom: (timelineIndex: number, text: string) => Promise<void>;
  /** Explicitly inject one queued text-only message into the active local run. */
  steerQueued: (id: string) => Promise<void>;
  removeQueued: (id: string) => void;
  setApprovalPolicy: (mode: ApprovalPolicy) => void;
  /** Shift+Tab: advance to the next permission mode in the cycle. */
  cycleApprovalPolicy: () => void;
  setCollaborationMode: (mode: CollaborationMode) => void;
  decidePlan: (
    planId: string,
    decision: { action: "implement"; context: PlanImplementationContext } |
      { action: "continue_planning"; feedback?: string },
  ) => Promise<void>;
  setOutputStyle: (style: string) => void;
  toggleTerminal: () => void;
  setTerminalOpen: (open: boolean) => void;
  /** Make a folder the current project (seeding the next session) and open a
   *  fresh terminal tab rooted in it. With no `path`, asks the OS for a
   *  folder first; cancelling the picker is a no-op. */
  openProjectTerminal: (path?: string) => Promise<void>;
  setMcpOpen: (open: boolean) => void;
  setSshOpen: (open: boolean) => void;
  /** Open/close the unified Settings modal, optionally jumping to a section. */
  setSettingsOpen: (open: boolean, section?: SettingsSection) => void;
  setPaletteOpen: (open: boolean) => void;
  togglePalette: () => void;
  /** Check for, download, verify, and stage a newer version (no install yet). */
  checkForUpdate: () => Promise<UpdateCheckResult>;
  /** Drain active work, install the staged update, and relaunch. */
  applyUpdate: () => Promise<void>;
  /** Dismiss the "updated to vX" confirmation. */
  dismissJustUpdated: () => void;
  /** Clear the transient error banner. */
  dismissError: () => void;
  /** Show a transient success/info toast that auto-dismisses. */
  flashNotice: (message: string) => void;
  /** Clear the transient notice toast. */
  dismissNotice: () => void;
  /** Clear the transient warning toast. */
  dismissWarning: () => void;
  /** Hide the current activity reward and remember that it was seen. */
  dismissActivityReward: () => void;
  /** Hide the failed/stopped terminal banner for a specific run. */
  dismissFailedRun: (runId: string) => void;
  toggleSidebar: () => void;
  setSidebarCollapsed: (collapsed: boolean) => void;
  cancelActive: () => Promise<void>;
  resolvePermission: (option: string) => Promise<void>;
  /** `/btw` — ask a side question (forked, no main-run interruption). */
  askSideQuestion: (question: string) => Promise<void>;
  /** Dismiss the side-question overlay (and drop a still-pending answer). */
  dismissSideQuestion: () => void;
}

// Session transitions (start / open / end) are async and can take 10–20s over
// SSH. Every transition bumps this epoch; a continuation that awakes to find a
// newer epoch was superseded (user cancelled, hit ⌘N, or clicked another
// conversation) and must abandon — cleaning up any tunnel it opened — instead
// of clobbering the newer state.
let sessionEpoch = 0;
const nextSessionEpoch = () => ++sessionEpoch;
const epochStale = (epoch: number) => epoch !== sessionEpoch;

/** Shown when archive/delete would destroy a conversation whose run is still
 *  streaming. Switching chats is always safe — sessions keep running in the
 *  background — but archive/delete tears the session down for real. */
const BUSY_SESSION_MESSAGE =
  "Clark is still working in this chat — stop the run first (⌘.), then archive or delete it.";

/** One live session in the pool. Every opened conversation gets an entry that
 *  keeps its provider session (and any streaming run) alive independently of
 *  which conversation is displayed — there is no limit on how many run at
 *  once. Non-reactive on purpose: the UI renders only the ACTIVE session's
 *  snapshot (mirrored into the store) plus the lightweight `runningIds` list. */
interface LiveEntry {
  session: Session;
  /** Latest raw engine snapshot for this session (no history prefix). */
  live: Snapshot;
  /** Restored transcript this session was reopened on top of, if any. */
  historyPrefix: Snapshot | null;
  /** The SSH tunnel backing this session (remote projects) — torn down only
   *  when the session closes, never on a switch. */
  remote: RemoteInfo | null;
  remoteHost: string | null;
  /** Project folder captured at open time, so background persistence and UI
   *  actions cannot drift when the new-session folder setting changes. */
  projectRoot: string | null;
  /** Follow-ups typed while this conversation's run was streaming. */
  queued: QueuedMessage[];
  // Per-session bookkeeping for the shared snapshot handler.
  lastPersist: number;
  prevBusy: boolean;
  dispatching: boolean;
  /** A prompt invoke has begun but its first running snapshot may not have
   *  arrived yet. Closes the frontend side of the update/start race. */
  starting: boolean;
  autoResolvedId: string | null;
  notifiedPermId: string | null;
}

/** The pool of live sessions, keyed by conversation id. */
const liveSessions = new Map<string, LiveEntry>();

/** Freeze the model a local conversation was created/reopened with. Without
 * this snapshot, chats with no explicit override keep following the mutable
 * new-chat default, so changing that default makes every such chat appear to
 * switch models retroactively. */
function pinChatModel(
  get: () => SessionState,
  set: (partial: Partial<SessionState>) => void,
  id: string,
  settings: LocalAgentSettings,
): void {
  const current = get().chatModels;
  if (current[id]) return;
  const next = {
    ...current,
    [id]: {
      model: settings.model,
      reasoningEffort: normalizeReasoningEffort(settings.model, settings.reasoningEffort),
    },
  };
  saveChatModels(next);
  set({ chatModels: next });
}

function newLiveEntry(
  session: Session,
  init: Pick<LiveEntry, "historyPrefix" | "remote" | "remoteHost" | "projectRoot">,
): LiveEntry {
  return {
    session,
    live: { ...emptySnapshot(), session: session.id },
    queued: [],
    lastPersist: 0,
    prevBusy: false,
    dispatching: false,
    starting: false,
    autoResolvedId: null,
    notifiedPermId: null,
    ...init,
  };
}

/** The entry's displayable snapshot: restored history + live turns. */
function mergedOf(entry: LiveEntry): Snapshot {
  return entry.historyPrefix ? mergeHistory(entry.historyPrefix, entry.live) : entry.live;
}

/** Close one live session for real: drop the host-side provider (killing any
 *  agent loop) and its SSH tunnel. Only archive/delete/sign-out do this. */
function closeLiveSession(bridge: CoreBridge | null, id: string): void {
  const entry = liveSessions.get(id);
  if (!entry) return;
  liveSessions.delete(id);
  void bridge?.closeSession?.(id);
  if (entry.remote) void sshDisconnect(entry.remote.id);
}

/** Bring up the exec-server + tunnel for `host`. Throws a readable error if the
 *  host is incomplete or the connection fails (unreachable, arch mismatch, …). */
async function openRemote(host: SshHost, projectRoot = host.remoteRoot): Promise<RemoteInfo> {
  if (!hostReady(host)) {
    throw new Error("This host needs a remote folder and an exec-server binary path.");
  }
  return sshConnect(host.host.trim(), projectRoot.trim(), host.binaryPath.trim());
}

// Chat history is cloud-only. Snapshots are cached in memory for the app's
// lifetime (never persisted to disk) so re-opening a conversation within a
// session is instant; a cold start re-fetches from the cloud. The conversation
// LIST lives in the store's `conversations` and is populated from the cloud on
// init/sign-in (see `syncCloudIndex`).
const snapshotCache = new Map<string, Snapshot>();

const UPDATE_DRAIN_POLL_MS = 250;

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function liveUpdateBlockerCount(): number {
  return updateDrainBlockerCount(
    [...liveSessions.values()].map((entry) => ({
      live: entry.live,
      queuedCount: entry.queued.length,
      dispatching: entry.dispatching,
      starting: entry.starting,
    })),
  );
}

let updateMenuListenerInstalled = false;
let settingsMenuListenerInstalled = false;
let updateTimersInstalled = false;

const ACTIVITY_REWARD_SEEN_PREFIX = "clark.activity-reward.seen.v1";

function activityRewardSeenKey(auth: AuthSession | null, reward: ActivityReward): string | null {
  const account = auth?.user.email?.trim() || auth?.user.name.trim();
  return account ? `${ACTIVITY_REWARD_SEEN_PREFIX}.${encodeURIComponent(account)}.${reward.id}` : null;
}

function hasSeenActivityReward(auth: AuthSession | null, reward: ActivityReward): boolean {
  const key = activityRewardSeenKey(auth, reward);
  if (!key) return true;
  try {
    return localStorage.getItem(key) === "1";
  } catch {
    return false;
  }
}

function markActivityRewardSeen(auth: AuthSession | null, reward: ActivityReward): void {
  const key = activityRewardSeenKey(auth, reward);
  if (!key) return;
  try {
    localStorage.setItem(key, "1");
  } catch {
    // This is presentation-only; an in-memory dismissal still avoids a loop.
  }
}

async function bindCloudTrajectory(
  bridge: CoreBridge,
  session: Session,
  meta: ConversationMeta,
  auth: AuthSession | null,
  metadata: Record<string, unknown>,
): Promise<void> {
  // Browser preview/dev bridges have no native cloud sink. Production Tauri
  // always implements it and requires authenticated Clark cloud credentials.
  if (!bridge.configureCloudTrajectory) return;
  const creds = cloudCreds(auth);
  if (!creds) {
    throw new Error("Clark cloud is required to start or resume a coding session.");
  }
  const repositoryFingerprint = meta.project
    ? await repositoryFingerprintForRoot(meta.project)
    : null;
  const config: CloudTrajectoryConfig = {
    endpoint: creds.endpoint,
    token: creds.token,
    title: meta.title,
    provider: meta.provider,
    project: meta.project,
    repositoryFingerprint: repositoryFingerprint ?? undefined,
    remoteHost: meta.remoteHost,
    mode: meta.mode,
    metadata,
  };
  await bridge.configureCloudTrajectory(session.id, config);
}

/** Cloud-first snapshot lookup: the in-memory cache, else a `cloudGet` (settled
 *  so a persisted mid-run transcript never reopens "Thinking…"), else null. */
async function fetchSnapshot(id: string, auth: AuthSession | null): Promise<Snapshot | null> {
  const cached = snapshotCache.get(id);
  if (cached) return cached;
  const creds = cloudCreds(auth);
  if (!creds) return null;
  try {
    const cloud = await cloudGet(creds, id);
    if (cloud) {
      const settled = settleRuns(cloud);
      snapshotCache.set(id, settled);
      return settled;
    }
  } catch {
    /* offline / backend down — caller falls back to a fresh session */
  }
  return null;
}

/** Merge a cloud conversation list over the in-memory one: cloud entries win,
 *  but any in-memory-only entries (e.g. just migrated, push not yet landed) are
 *  preserved so they don't flash out of the sidebar. */
function mergeConversations(
  cloud: ConversationMeta[],
  local: ConversationMeta[],
): ConversationMeta[] {
  const byId = new Map<string, ConversationMeta>();
  for (const c of local) byId.set(c.id, c);
  for (const c of cloud) byId.set(c.id, c);
  return [...byId.values()];
}

const bootAuth = loadAuthSession();

export const useSessionStore = create<SessionState>((set, get) => ({
  bridge: null,
  providers: [],
  activeProvider: null,
  session: null,
  snapshot: emptySnapshot(),
  connecting: false,
  error: null,
  notice: null,
  warning: null,
  dismissedFailedRuns: [],
  auth: bootAuth,
  attachments: [],
  conversations: [],
  conversationsLoading: !!bootAuth,
  historyPrefix: null,
  runningIds: [],
  selectedConversationIds: new Set<string>(),
  opening: null,
  composerPrefill: null,
  localSettings: loadLocalSettings(),
  chatModels: loadChatModels(),
  projectMode: "local",
  selectedHostId: loadSshHosts()[0]?.id ?? null,
  activeRemote: null,
  activeRemoteHost: null,
  activeProjectRoot: null,
  memoriesEnabled: loadMemoriesEnabled(),
  browserEnabled: loadBrowserEnabled(),
  orchestrationEnabled: loadOrchestrationEnabled(),
  memoryStatus: null,
  memoryViewerOpen: false,
  loadingMemory: false,
  memoryOverview: null,
  globalMemoryOverview: null,
  recentProjects: loadRecentProjects(),
  queued: [],
  approvalPolicy: loadApprovalPolicy(),
  collaborationMode: loadCollaborationMode(),
  outputStyle: loadOutputStyle(),
  terminalOpen: false,
  terminalLaunch: null,
  mcpOpen: false,
  sshOpen: false,
  settingsOpen: false,
  settingsSection: "general",
  paletteOpen: false,
  sideQuestion: null,
  sidebarCollapsed: false,
  billing: null,
  loadingBilling: false,
  activityReward: null,
  update: null,
  updateProgress: null,
  updateChecking: false,
  updateApplying: false,
  updateWaiting: false,
  justUpdatedTo: null,

  checkForUpdate: async () => {
    const ready = get().update;
    if (ready) return { status: "ready", update: ready };
    if (get().updateChecking || get().updateProgress) return { status: "busy" };

    set({ updateChecking: true });
    try {
      const result = await checkAndStageUpdate((progress) => set({ updateProgress: progress }));
      if (result.status === "ready") set({ update: result.update });
      return result;
    } finally {
      set({ updateChecking: false, updateProgress: null });
    }
  },
  applyUpdate: async () => {
    if (!get().update || get().updateWaiting || get().updateApplying) return;
    set({ updateWaiting: true });
    let installed = false;
    try {
      // Let current runs and already-queued follow-ups finish first. New sends
      // are rejected while `updateWaiting` is true, but permission responses and
      // cancellation remain available so a blocked run can still settle.
      while (get().connecting || liveUpdateBlockerCount() > 0) {
        await delay(UPDATE_DRAIN_POLL_MS);
      }

      // Close the final native race: latch prompt starts, then wait for any run
      // that entered just before the latch to release its RAII guard.
      while ((await beginUpdateDrain()) > 0) {
        await delay(UPDATE_DRAIN_POLL_MS);
      }

      if (!(await flushCloudPuts())) {
        throw new Error("Clark Code could not save the final conversation state; update postponed.");
      }

      set({ updateWaiting: false, updateApplying: true });
      await installStagedUpdate();
      installed = true;
      await relaunchApp();
      // The process normally exits before this fires. If the platform accepted
      // the request but did not relaunch, release the blocking overlay/latch.
      await delay(1500);
      throw new Error("Clark Code did not relaunch. Quit and reopen it to finish the update.");
    } catch (error) {
      await cancelUpdateDrain().catch(() => {});
      set({
        updateWaiting: false,
        updateApplying: false,
        ...(installed ? { update: null } : {}),
        error: String(error),
      });
    }
  },
  dismissJustUpdated: () => set({ justUpdatedTo: null }),
  dismissError: () => set({ error: null }),
  flashNotice: (message) => set({ notice: message }),
  dismissNotice: () => set({ notice: null }),
  dismissWarning: () => set({ warning: null }),
  dismissActivityReward: () => {
    const reward = get().activityReward;
    if (reward) markActivityRewardSeen(get().auth, reward);
    set({ activityReward: null });
  },
  dismissFailedRun: (runId) =>
    set((s) =>
      s.dismissedFailedRuns.includes(runId)
        ? s
        : { dismissedFailedRuns: [...s.dismissedFailedRuns, runId] },
    ),

  loadBilling: async () => {
    const creds = cloudCreds(get().auth);
    if (!creds) {
      set({ billing: null, activityReward: null });
      return;
    }
    set({ loadingBilling: true });
    try {
      const billing = await billingMe(creds);
      const reward = latestActivityReward(billing);
      const current = get().activityReward;
      const activityReward =
        current ?? (reward && !hasSeenActivityReward(get().auth, reward) ? reward : null);
      set({ billing, loadingBilling: false, activityReward });
    } catch {
      set({ loadingBilling: false });
    }
  },

  init: async () => {
    // Native chrome and self-update are app-lifecycle concerns, not provider
    // concerns. Install them before provider discovery so a broken provider can
    // never suppress Settings or the recovery update path. Guards also prevent
    // React Strict Mode's development double-mount from duplicating timers.
    if (!settingsMenuListenerInstalled) {
      settingsMenuListenerInstalled = true;
      void onSettingsMenuRequested(() => get().setSettingsOpen(true)).catch(() => {
        settingsMenuListenerInstalled = false;
      });
    }
    if (!updateMenuListenerInstalled) {
      updateMenuListenerInstalled = true;
      void onUpdateMenuRequested(() => {
        void (async () => {
          const result = await get().checkForUpdate();
          if (result.status === "ready") {
            get().flashNotice(
              `Clark Code ${result.update.version} is downloaded and ready to install.`,
            );
          } else if (result.status === "up-to-date") {
            get().flashNotice("Clark Code is already up to date.");
          } else if (result.status === "busy") {
            get().flashNotice("Clark Code is already checking for or downloading an update.");
          } else if (result.status === "error") {
            get().flashNotice(
              "Clark Code couldn't check for updates. Check your connection and try again.",
            );
          }
        })();
      }).catch(() => {
        updateMenuListenerInstalled = false;
      });
    }
    if (!updateTimersInstalled) {
      updateTimersInstalled = true;
      // Downloads + verifies + stages in the background; the UI surfaces the
      // ready action. Retrying every six hours stays silent on network errors.
      setTimeout(() => void get().checkForUpdate(), 4000);
      setInterval(() => void get().checkForUpdate(), 6 * 60 * 60 * 1000);
      void consumeJustUpdated().then((version) => {
        if (version) set({ justUpdatedTo: version });
      });
    }

    try {
      const bridge = await getBridge();
      const providers = await bridge.listProviders();
      // Native trajectory sync hit a 401 mid-retry: re-mint the Clark JWT from
      // the Google refresh token and push it back down — the retry loop reads
      // the token per attempt, so the run self-heals without any UI. Single-
      // flight: retries can raise the event repeatedly during one refresh.
      // A failed refresh mid-run must NOT sign the user out; the append just
      // exhausts its retries and surfaces as a soft cloud-sync warning.
      let refreshingCloudToken = false;
      bridge.onCloudAuthExpired?.(() => {
        if (refreshingCloudToken) return;
        refreshingCloudToken = true;
        void (async () => {
          try {
            const auth = get().auth;
            const refreshed = auth ? await refreshAuthSession(auth) : null;
            if (refreshed) {
              set({ auth: refreshed });
              if (refreshed.clark.token) {
                await bridge.updateCloudToken?.(refreshed.clark.token);
              }
            }
          } finally {
            refreshingCloudToken = false;
          }
        })();
      });
      // Best-effort cloud sync failed for part of a run — the run keeps going;
      // show a non-blocking warning instead of the fatal error banner.
      bridge.onCloudSyncWarning?.((message) => set({ warning: message }));
      // The host re-emits a fully cloned snapshot on every streamed token (tens
      // per second), for EVERY live session concurrently. Two throttles keep
      // that from melting the UI:
      //   • render — only the ACTIVE session renders, coalesced to at most one
      //     React update per animation frame;
      //   • persist — each session writes its transcript at most ~2×/sec while
      //     streaming, and always once it goes idle so nothing is lost.
      const raf: (cb: () => void) => void =
        typeof requestAnimationFrame !== "undefined"
          ? (cb) => requestAnimationFrame(() => cb())
          : (cb) => void setTimeout(cb, 16);
      // Buffer the RAW live snapshot and merge with the entry's history prefix
      // at flush time; a switch mid-frame drops the stale flush (the snapshot's
      // session tag no longer matches the active conversation).
      let pending: Snapshot | null = null;
      let rafScheduled = false;
      const flushRender = () => {
        rafScheduled = false;
        if (!pending) return;
        const live = pending;
        pending = null;
        const active = get().session;
        if (!active || live.session !== active.id) return;
        const entry = liveSessions.get(active.id);
        const prefix = entry?.historyPrefix ?? null;
        const merged = prefix ? mergeHistory(prefix, live) : live;
        // Push fan-out state into its own (deduped) store on the SAME coalesced
        // frame as the render — not per raw event — so an active swarm's tiles
        // update at most once per animation frame instead of on every telemetry
        // event (the compositing/opacity churn behind the fan-out flicker).
        syncFanOut(merged.fan_out);
        set({ snapshot: merged });
      };
      let lastBilling = 0;
      // Route each engine snapshot to its live-session entry (any number can
      // stream at once): render if active, persist, notify, auto-approve, and
      // drain queued follow-ups — all per session.
      bridge.subscribe((live) => {
        const id = live.session;
        const entry = id ? liveSessions.get(id) : undefined;
        // No entry: the clean announce emitted before the store registers the
        // session, or a trailing event after a close — nothing to route to.
        if (!id || !entry) return;
        entry.live = live;
        const snapshot = entry.historyPrefix ? mergeHistory(entry.historyPrefix, live) : live;
        const busyNow = isBusy(live);
        const justSettled = entry.prevBusy && !busyNow;
        const isActive = get().session?.id === id;

        if (isActive) {
          pending = live;
          if (!rafScheduled) {
            rafScheduled = true;
            raf(flushRender);
          }
        }

        // Persist + sidebar meta: throttled while streaming, immediate when
        // idle. While busy the work runs in a macrotask AFTER the frame commits
        // — JSON.stringify of a long transcript was a source of 100ms+ hitches.
        if (hasContent(snapshot)) {
          const now = Date.now();
          if (!busyNow || now - entry.lastPersist >= 2000) {
            entry.lastPersist = now;
            const persistPrefixLen = entry.historyPrefix?.timeline.length ?? 0;
            const persist = () => {
              // Cache the latest snapshot in memory (never to disk); the cloud
              // push below is the durable copy.
              snapshotCache.set(id, snapshot);
              const prev = get().conversations.find((c) => c.id === id);
              // Project folder is the remote root for a remote session, else the
              // folder captured when this session opened.
              const project =
                entry.session.provider === "local"
                  ? entry.projectRoot || undefined
                  : undefined;
              // Only advance updatedAt when the timeline actually grew past the
              // restored prefix — i.e. real new activity. Merely opening/resuming a
              // conversation replays its transcript without adding turns, so its
              // order (and its whole project group's) must stay put in the sidebar.
              const grew = snapshot.timeline.length > persistPrefixLen;
              const meta: ConversationMeta = {
                id,
                // A manual rename wins over the auto-derived title forever.
                title: prev?.titleLocked && prev.title ? prev.title : deriveTitle(snapshot),
                provider: entry.session.provider,
                mode: entry.session.mode,
                project: project ?? prev?.project,
                remoteHost: entry.remoteHost ?? prev?.remoteHost,
                titleLocked: prev?.titleLocked,
                createdAt: prev?.createdAt ?? Date.now(),
                updatedAt: grew ? Date.now() : (prev?.updatedAt ?? Date.now()),
                archived: prev?.archived,
              };
              // Update the in-memory sidebar list only when visible bits changed
              // (avoids a re-render per streamed save). Newest-first.
              if (
                !prev ||
                prev.title !== meta.title ||
                prev.updatedAt !== meta.updatedAt ||
                prev.project !== meta.project
              ) {
                set({ conversations: [meta, ...get().conversations.filter((c) => c.id !== meta.id)] });
              }
              // Mirror to Clark on the same throttle as local persistence:
              // ~every 2s while streaming (so mobile/web can watch the run
              // live and show a running indicator), and immediately when the
              // turn settles. Coalesced + single-flight + idempotent (see
              // cloudHistory), so the streaming pushes stay cheap and ordered.
              const creds = cloudCreds(get().auth);
              if (creds) scheduleCloudPut(creds, meta, snapshot, busyNow ? "running" : "idle");
            };
            if (busyNow) setTimeout(persist, 0);
            else persist();
          }
        }

        if (busyNow) entry.dispatching = false; // a run is active again — clear the drain guard

        // Per-conversation "Working…" indicator for the sidebar.
        if (busyNow !== entry.prevBusy) {
          const ids = get().runningIds;
          set({ runningIds: busyNow ? [...ids, id] : ids.filter((r) => r !== id) });
        }

        // Native notification on the busy→idle edge (desktop only, and only when
        // the window is unfocused — see notify()).
        if (justSettled) {
          const failedRun = Object.values(live.runs).some((r) => r.status === "failed");
          const title = get().conversations.find((c) => c.id === id)?.title;
          if (failedRun) {
            void notify("Run failed", title ? `“${title}” ended with an error.` : "The agent ended unexpectedly.");
          } else {
            void notify("Clark finished", title ? `“${title}” is ready for review.` : "Your task is ready for review.");
          }
        }
        entry.prevBusy = busyNow;

        // Refresh the credit balance shortly after a turn settles so the credit
        // banner reflects spend (throttled — billing is a network call).
        if (!busyNow) {
          const now = Date.now();
          if (justSettled || now - lastBilling > 15000) {
            lastBilling = now;
            void get().loadBilling();
          }
        }

        // Auto-approve the pending permission per the current policy (Full access
        // grants everything; "Approve for me" grants all but destructive-looking
        // actions). Guarded so each request is answered exactly once — works for
        // background sessions too, so a run never stalls just because its
        // conversation isn't on screen.
        const pend = live.pending_permission;
        if (pend) {
          if (pend.id !== entry.autoResolvedId && wouldAutoApprove(get().approvalPolicy, pend)) {
            const opt = pickAllowOption(pend);
            if (opt) {
              entry.autoResolvedId = pend.id;
              bridge
                .respond(id, { kind: "permission", request: pend.id, option: opt.id })
                .catch((e) => set({ error: String(e) }));
            }
          } else if (pend.id !== entry.notifiedPermId && !wouldAutoApprove(get().approvalPolicy, pend)) {
            // The gate will actually block for the user — ping them.
            entry.notifiedPermId = pend.id;
            void notify("Approval needed", pend.title || "Clark is waiting for your approval.");
          }
        } else {
          entry.autoResolvedId = null;
          entry.notifiedPermId = null;
        }

        // Drain this conversation's next queued message whenever idle and
        // unblocked. Draining on every idle snapshot (not just the busy→idle
        // edge) means a permission prompt open at the finish moment never
        // strands the queue; `dispatching` prevents a double-send before the
        // new run shows up as busy.
        if (!busyNow && !live.pending_permission && !entry.dispatching && entry.queued.length > 0) {
          const [next, ...rest] = entry.queued;
          entry.dispatching = true;
          entry.queued = rest;
          if (isActive) set({ queued: rest });
          bridge
            .prompt(id, [{ type: "text", text: next.text }], next.uploads)
            .catch((e) => {
              set({ error: String(e) });
              entry.dispatching = false;
            });
        }
      });
      set({
        bridge,
        providers,
        activeProvider: providers[0]?.id ?? null,
      });
      // Best-effort: ensure a Clark Code key exists, migrate any residual local
      // chats into the cloud (one-time), pull cloud history, and load the credit
      // balance. All no-op offline / signed out.
      void get().ensureCodeKey();
      get().migrateLocalToCloud();
      void get().syncCloudIndex();
      void get().loadBilling();
    } catch (e) {
      set({ error: String(e) });
    }
  },

  ensureCodeKey: async () => {
    // Already have a key, or can't provision (browser / signed out).
    if (get().localSettings.apiKey.trim()) return;
    const creds = cloudCreds(get().auth);
    if (!creds) return;
    try {
      const key = await provisionCodeKey(creds);
      if (key) get().setLocalSettings({ apiKey: key });
    } catch {
      /* offline / backend not deployed — onboarding still works, the key is
         re-attempted on the next sign-in / session start */
    }
  },

  syncCloudIndex: async () => {
    const creds = cloudCreds(get().auth);
    if (!creds) {
      set({ conversationsLoading: false });
      return;
    }
    try {
      const remote = await cloudList(creds);
      // Cloud is authoritative; keep only in-memory-only entries (a just-migrated
      // chat whose push hasn't landed yet) so they don't flash out of the list.
      set({
        conversations: mergeConversations(remote, get().conversations),
        conversationsLoading: false,
      });
    } catch {
      // Offline / backend down — leave whatever's in memory, stop the spinner.
      set({ conversationsLoading: false });
    }
  },

  migrateLocalToCloud: () => {
    const creds = cloudCreds(get().auth);
    if (!creds) return; // no cloud target yet — keep local data for a later sign-in
    const drained = drainLocalHistory();
    if (drained.length === 0) return;
    for (const d of drained) {
      // Seed the cache so migrated chats open instantly (settled — a drained
      // transcript may have been persisted mid-run), then upload snapshot +
      // archived state (idempotent; the server's rev guard won't clobber newer).
      snapshotCache.set(d.meta.id, settleRuns(d.snapshot));
      scheduleCloudPut(creds, d.meta, d.snapshot);
      if (d.archived) void cloudSetArchived(creds, d.meta.id, true).catch(() => {});
    }
    const existing = new Set(get().conversations.map((c) => c.id));
    const add = drained.map((d) => d.meta).filter((m) => !existing.has(m.id));
    if (add.length) set({ conversations: [...add, ...get().conversations] });
  },

  selectProvider: (id) => set({ activeProvider: id }),

  setLocalSettings: (patch) => {
    const next = { ...get().localSettings, ...patch };
    saveLocalSettings(next);
    set({ localSettings: next });
  },

  setProjectMode: (mode) => set({ projectMode: mode, error: null }),
  setSelectedHostId: (id) => set({ selectedHostId: id }),

  setProjectFolder: (path) => {
    get().setLocalSettings({ cwd: path });
    set({ recentProjects: addRecentProject(path), memoryStatus: null, memoryOverview: null });
  },

  pickProjectFolder: async () => {
    try {
      const picked = await pickFolder(get().localSettings.cwd || undefined);
      if (picked) get().setProjectFolder(picked);
    } catch (e) {
      set({ error: String(e) });
    }
  },

  setMemoriesEnabled: (on) => {
    saveMemoriesEnabled(on);
    set({ memoriesEnabled: on });
  },

  setBrowserEnabled: (on) => {
    saveBrowserEnabled(on);
    set({ browserEnabled: on });
  },

  setOrchestrationEnabled: (on) => {
    saveOrchestrationEnabled(on);
    set({ orchestrationEnabled: on });
  },

  loadMemory: async () => {
    const { bridge, localSettings: s, activeProjectRoot, activeRemote } = get();
    const cwd = activeProjectRoot?.trim() || s.cwd.trim();
    const remote = activeRemote
      ? { ws_url: activeRemote.ws_url, token: activeRemote.token }
      : null;
    if (!bridge?.listMemory) {
      set({ memoryOverview: null, globalMemoryOverview: null });
      return;
    }
    set({ loadingMemory: true });
    try {
      // Project scope needs a folder; global scope is per-user (always available).
      // minLoadDuration holds the loading flag for one spin so the refresh icon
      // animates — a local disk read settles in a single frame and React never
      // paints the spinner, so the click looks frozen. Slower reads already spin.
      const [memoryOverview, globalMemoryOverview] = await minLoadDuration(
        Promise.all([
          cwd ? bridge.listMemory(cwd, remote) : Promise.resolve(null),
          bridge.listGlobalMemory?.() ?? Promise.resolve(null),
        ]),
      );
      set({ loadingMemory: false, memoryOverview, globalMemoryOverview, memoryStatus: null });
    } catch (e) {
      await minLoadDuration(Promise.reject(e));
      set({ loadingMemory: false, memoryStatus: `Could not read memory: ${String(e)}` });
    }
  },

  toggleMemoryViewer: () => {
    const open = !get().memoryViewerOpen;
    set({ memoryViewerOpen: open });
    if (open) void get().loadMemory();
  },

  setMemoryViewerOpen: (open) => set({ memoryViewerOpen: open }),

  signIn: async () => {
    const auth = await signInWithGoogle();
    // Start from an empty list for the new account; the cloud fetch below is the
    // authoritative source (a different account never inherits the prior list).
    set({ auth, billing: null, activityReward: null, conversations: [], conversationsLoading: true });
    // Provision the Clark Code key; migrate any residual local chats into this
    // account's cloud (one-time — self-cleaning); then pull the cloud list.
    void get().ensureCodeKey();
    get().migrateLocalToCloud();
    void get().syncCloudIndex();
    void get().loadBilling();
  },

  signOutAuth: () => {
    authSignOut();
    get().endSession({ force: true });
    // Drop the in-memory history entirely so the signed-out (and any next)
    // account starts clean.
    snapshotCache.clear();
    set({ auth: null, billing: null, activityReward: null, conversations: [], conversationsLoading: false });
  },

  startBlockedReason: () => {
    const { activeProvider, projectMode, localSettings, selectedHostId } = get();
    // Non-local providers (cloud) need no local folder/host — always ready.
    if (activeProvider !== "local") return null;
    if (projectMode === "remote") {
      const host = loadSshHosts().find((h) => h.id === selectedHostId);
      if (!host) return "Add a remote host.";
      if (!hostReady(host)) return "This host needs a folder and exec-server binary.";
      return null;
    }
    return localSettingsReady(localSettings);
  },

  startSession: async () => {
    const { bridge, activeProvider, auth } = get();
    if (!bridge || !activeProvider) return;
    const epoch = nextSessionEpoch();
    const isLocal = activeProvider === "local";
    const isRemote = isLocal && get().projectMode === "remote";
    const startHost = isRemote
      ? (loadSshHosts().find((h) => h.id === get().selectedHostId)?.host.trim() ?? null)
      : null;
    set({
      connecting: true,
      error: null,
      opening: { id: null, kind: "start", title: "New session", remoteHost: startHost },
    });
    let remote: RemoteInfo | null = null;
    let nativeSession: Session | null = null;
    try {
      // Make sure a Clark Code key has been minted before the local provider
      // needs it (covers the case where sign-in's background provision is still
      // in flight or failed).
      if (isLocal) await get().ensureCodeKey();
      const localSettings = get().localSettings;

      // Remote: bring up the exec-server + tunnel, then connect the provider to
      // run its tools there. Local: run the loop on this machine. Other
      // providers connect with the signed-in Clark config, no embedded creds.
      let config;
      let options;
      let remoteHost: string | null = null;
      const collaboration_mode = get().collaborationMode;
      const mode = get().approvalPolicy;
      if (isRemote) {
        const host = loadSshHosts().find((h) => h.id === get().selectedHostId);
        if (!host) throw new Error("Pick a remote host first, or add one.");
        remote = await openRemote(host);
        remoteHost = host.host.trim();
        config = localConnectConfig(localSettings, remoteTarget(remote));
        options = { cwd: remote.cwd, mode, collaboration_mode };
      } else if (isLocal) {
        config = localConnectConfig(localSettings);
        options = { cwd: localSettings.cwd.trim(), mode, collaboration_mode };
      } else {
        config = { endpoint: auth?.clark.endpoint, auth_token: auth?.clark.token };
        options = {};
      }

      // Superseded (cancel / another open) while connecting → abandon quietly.
      if (epochStale(epoch)) {
        if (remote) void sshDisconnect(remote.id);
        return;
      }

      await bridge.connect(activeProvider, config);
      const session = await bridge.newSession(activeProvider, options);
      nativeSession = session;
      if (epochStale(epoch)) {
        void bridge.closeSession?.(session.id);
        if (remote) void sshDisconnect(remote.id);
        return;
      }
      const project = isLocal
        ? (isRemote ? remote?.cwd : localSettings.cwd.trim()) || undefined
        : undefined;
      const projectRoot = liveProjectRoot(session, project ?? null);
      const now = Date.now();
      const conversationMeta: ConversationMeta = {
        id: session.id,
        title: "New conversation",
        provider: activeProvider,
        project: projectRoot ?? project,
        remoteHost: remoteHost ?? undefined,
        mode: session.mode,
        createdAt: now,
        updatedAt: now,
      };
      if (isLocal) pinChatModel(get, set, session.id, localSettings);
      await bindCloudTrajectory(
        bridge,
        session,
        conversationMeta,
        get().auth,
        {
          projectMode: get().projectMode,
          model: isLocal ? localSettings.model : undefined,
          reasoningEffort: isLocal ? localSettings.reasoningEffort : undefined,
          approvalPolicy: get().approvalPolicy,
          outputStyle: get().outputStyle,
          memoriesEnabled: get().memoriesEnabled,
          browserEnabled: get().browserEnabled,
        },
      );
      if (isLocal && !isRemote && localSettings.cwd.trim()) {
        set({ recentProjects: addRecentProject(localSettings.cwd.trim()) });
      }
      // Register in the live-session pool — other sessions keep running
      // untouched; this one joins them and becomes the displayed conversation.
      liveSessions.set(
        session.id,
        newLiveEntry(session, {
          historyPrefix: null,
          remote,
          remoteHost,
          projectRoot,
        }),
      );
      nativeSession = null;
      set({
        session,
        snapshot: emptySnapshot(),
        connecting: false,
        opening: null,
        historyPrefix: null,
        queued: [],
        conversations: [
          conversationMeta,
          ...get().conversations.filter((c) => c.id !== session.id),
        ],
        activeRemote: remote,
        activeRemoteHost: remoteHost,
        activeProjectRoot: projectRoot,
      });
    } catch (e) {
      // Brought up a tunnel but failed afterward → tear it back down.
      if (nativeSession) void bridge.closeSession?.(nativeSession.id);
      if (remote) void sshDisconnect(remote.id);
      if (epochStale(epoch)) return;
      set({ error: String(e), connecting: false, opening: null });
    }
  },

  endSession: (opts) => {
    // Detach, don't destroy: the conversation's live session (and any streaming
    // run) stays in the pool, so ⌘N/"New chat" never cancels work — reopening
    // from the sidebar reattaches instantly. Bumping the epoch also cancels any
    // in-flight start/open (the OpeningScreen's Cancel): its continuation sees
    // a newer epoch and abandons, tearing down whatever tunnel it brought up.
    nextSessionEpoch();
    // Leaving the active conversation: drop any swarm panel so it doesn't linger
    // on the start screen or the next conversation opened.
    resetFanOut();
    for (const a of get().attachments) if (a.previewUrl) URL.revokeObjectURL(a.previewUrl);
    if (opts?.force) {
      // Sign-out: tear down every live session for real.
      const bridge = get().bridge;
      for (const id of [...liveSessions.keys()]) closeLiveSession(bridge, id);
      set({ runningIds: [] });
    }
    set({
      session: null,
      snapshot: emptySnapshot(),
      error: null,
      connecting: false,
      attachments: [],
      historyPrefix: null,
      opening: null,
      composerPrefill: null,
      queued: [],
      terminalOpen: false,
      sideQuestion: null,
      activeRemote: null,
      activeRemoteHost: null,
      activeProjectRoot: null,
      selectedConversationIds: new Set(),
    });
  },

  openConversation: async (id) => {
    const { bridge, activeProvider, auth, session, providers, localSettings } = get();
    if (!bridge || !activeProvider) return;
    // Already opening this one (double-click, impatient re-click) → no-op; the
    // in-flight open keeps its spinner.
    if (get().opening?.id === id) return;
    if (session?.id === id) return;
    // Leaving the current conversation: clear any swarm panel now. The reattach
    // branch re-syncs from the target's snapshot; a cold open stays cleared
    // until the resumed session streams its own fan-out.
    resetFanOut();
    // Supersede any in-flight open; live sessions are untouched by the epoch.
    const epoch = nextSessionEpoch();

    // Already live in the pool (streaming or idle) → instant reattach. No
    // reconnect, no loading screen, and absolutely nothing is torn down — the
    // conversation we're leaving keeps running in the background.
    const entry = liveSessions.get(id);
    if (entry) {
      if (entry.session.provider === "local") {
        pinChatModel(
          get,
          set,
          id,
          effectiveModelSettings(localSettings, get().chatModels, id),
        );
      }
      for (const a of get().attachments) if (a.previewUrl) URL.revokeObjectURL(a.previewUrl);
      const merged = mergedOf(entry);
      // Switching to an idle session emits no snapshot frame, so re-sync the
      // fan-out from the reattached snapshot instead of leaving the prior
      // conversation's swarm on screen.
      resetFanOut();
      syncFanOut(merged.fan_out);
      set({
        session: entry.session,
        snapshot: merged,
        historyPrefix: entry.historyPrefix,
        activeRemote: entry.remote,
        activeRemoteHost: entry.remoteHost,
        activeProjectRoot: liveProjectRoot(entry.session, entry.projectRoot),
        queued: entry.queued,
        attachments: [],
        connecting: false,
        opening: null,
        error: null,
        dismissedFailedRuns: [],
      });
      return;
    }

    const openingMeta = get().conversations.find((c) => c.id === id);
    set({
      connecting: true,
      error: null,
      dismissedFailedRuns: [],
      opening: {
        id,
        kind: "open",
        title: openingMeta?.title || "Conversation",
        remoteHost: openingMeta?.remoteHost ?? null,
      },
    });
    // Cloud-first: the transcript comes from the in-memory cache or a `cloudGet`.
    const restored = await fetchSnapshot(id, get().auth);
    let remote: RemoteInfo | null = null;
    let nativeSession: Session | null = null;
    try {
      const isLocal = activeProvider === "local";
      const canResume =
        providers.find((p) => p.id === activeProvider)?.capabilities.load_session ?? false;

      // A remote conversation reconnects its host (matched by SSH destination);
      // the saved host must still exist on this device.
      const wantRemote = isLocal && !!openingMeta?.remoteHost;
      const requestedProjectRoot = conversationProjectRoot(
        openingMeta?.project,
        localSettings.cwd,
      );
      let config;
      let options;
      let remoteHost: string | null = null;
      // Reopened local sessions resume in the composer's collaboration mode.
      // The model comes from the conversation's per-chat override when one was
      // set, else the global default — so reopening a chat that ran a different
      // model starts it on that model again, not the current default.
      const collaboration_mode = get().collaborationMode;
      const mode = get().approvalPolicy;
      const effSettings = effectiveModelSettings(localSettings, get().chatModels, id);
      if (isLocal) pinChatModel(get, set, id, effSettings);
      if (wantRemote) {
        const host = loadSshHosts().find((h) => h.host.trim() === openingMeta!.remoteHost);
        if (!host) {
          throw new Error(`Add the SSH host "${openingMeta!.remoteHost}" to reopen this remote conversation.`);
        }
        remote = await openRemote(
          host,
          conversationProjectRoot(openingMeta?.project, host.remoteRoot),
        );
        remoteHost = host.host.trim();
        config = localConnectConfig(effSettings, remoteTarget(remote));
        options = { cwd: remote.cwd, mode, collaboration_mode };
      } else if (isLocal) {
        if (!requestedProjectRoot) {
          throw new Error("This conversation has no project folder. Choose one before reopening it.");
        }
        config = localConnectConfig({ ...effSettings, cwd: requestedProjectRoot });
        options = { cwd: requestedProjectRoot, mode, collaboration_mode };
      } else {
        config = { endpoint: auth?.clark.endpoint, auth_token: auth?.clark.token };
        options = {};
      }

      // Providers that can't resume have no server-side context either: replay
      // the typed transcript so model history and the restored UI agree.
      if (!canResume && restored) {
        const resume = buildResumeTranscript(restored);
        if (resume) (options as SessionOptions).resume = resume;
      }

      // Superseded (cancel / another open) while connecting → abandon quietly.
      if (epochStale(epoch)) {
        if (remote) void sshDisconnect(remote.id);
        return;
      }

      await bridge.connect(activeProvider, config);
      // Providers that can't resume (the local agent has no server-side session)
      // reopen as a fresh session BOUND to the conversation id (the host keys
      // the session and tags its snapshots by it), so it doesn't fork into a
      // duplicate and events route back to this conversation.
      const opened = canResume
        ? await bridge.loadSession(activeProvider, id)
        : await bridge.newSession(activeProvider, options, id);
      nativeSession = opened;
      if (epochStale(epoch)) {
        void bridge.closeSession?.(opened.id);
        if (remote) void sshDisconnect(remote.id);
        return;
      }
      const trajectoryMeta: ConversationMeta = openingMeta ?? {
        id,
        title: "Conversation",
        provider: activeProvider,
        project: isLocal
          ? (wantRemote ? remote?.cwd : requestedProjectRoot) || undefined
          : undefined,
        remoteHost: remoteHost ?? undefined,
        mode: opened.mode,
        createdAt: Date.now(),
        updatedAt: Date.now(),
      };
      await bindCloudTrajectory(bridge, opened, trajectoryMeta, get().auth, {
        resumed: true,
        restoredSnapshot: restored !== null,
        projectMode: wantRemote ? "remote" : "local",
        model: isLocal ? effSettings.model : undefined,
        reasoningEffort: isLocal ? effSettings.reasoningEffort : undefined,
        approvalPolicy: get().approvalPolicy,
        outputStyle: get().outputStyle,
      });
      const projectRoot = liveProjectRoot(
        opened,
        (wantRemote ? remote?.cwd : requestedProjectRoot) || null,
      );
      liveSessions.set(
        id,
        newLiveEntry(opened, {
          historyPrefix: restored,
          remote,
          remoteHost,
          projectRoot,
        }),
      );
      nativeSession = null;
      set({
        session: opened,
        historyPrefix: restored,
        snapshot: restored ?? emptySnapshot(),
        connecting: false,
        opening: null,
        attachments: [],
        queued: [],
        activeRemote: remote,
        activeRemoteHost: remoteHost,
        activeProjectRoot: projectRoot,
      });
    } catch (e) {
      if (nativeSession) void bridge.closeSession?.(nativeSession.id);
      if (remote) void sshDisconnect(remote.id);
      if (epochStale(epoch)) return;
      set({ error: String(e), connecting: false, opening: null });
    }
  },

  archiveConversation: (id) => {
    // Soft-delete: flag it archived in the cloud (the transcript stays, so it
    // can be restored in full). Optimistic in-memory flag; PATCH to the cloud.
    // Archiving CLOSES the live session (unlike switching) — refuse mid-run.
    const entry = liveSessions.get(id);
    if (entry && isBusy(entry.live)) {
      get().flashNotice(BUSY_SESSION_MESSAGE);
      return;
    }
    closeLiveSession(get().bridge, id);
    const cleared = get().session?.id === id;
    set({
      conversations: get().conversations.map((c) => (c.id === id ? { ...c, archived: true } : c)),
      runningIds: get().runningIds.filter((r) => r !== id),
      ...(cleared
        ? {
            session: null,
            snapshot: emptySnapshot(),
            error: null,
            attachments: [],
            historyPrefix: null,
            queued: [],
            terminalOpen: false,
            activeRemote: null,
            activeRemoteHost: null,
            activeProjectRoot: null,
          }
        : {}),
    });
    const creds = cloudCreds(get().auth);
    if (creds) void cloudSetArchived(creds, id, true).catch(() => {});
  },

  restoreConversation: (id) => {
    set({
      conversations: get().conversations.map((c) => (c.id === id ? { ...c, archived: false } : c)),
    });
    const creds = cloudCreds(get().auth);
    if (creds) void cloudSetArchived(creds, id, false).catch(() => {});
  },

  deleteConversation: (id) => {
    // Hard delete: remove from the in-memory list + snapshot cache and delete the
    // cloud copy (best-effort — the list removal is what the user sees).
    // Deleting CLOSES the live session (unlike switching) — refuse mid-run.
    const entry = liveSessions.get(id);
    if (entry && isBusy(entry.live)) {
      get().flashNotice(BUSY_SESSION_MESSAGE);
      return;
    }
    closeLiveSession(get().bridge, id);
    snapshotCache.delete(id);
    const cleared = get().session?.id === id;
    set({
      conversations: get().conversations.filter((c) => c.id !== id),
      runningIds: get().runningIds.filter((r) => r !== id),
      ...(cleared
        ? {
            session: null,
            snapshot: emptySnapshot(),
            error: null,
            attachments: [],
            historyPrefix: null,
            queued: [],
            terminalOpen: false,
            activeRemote: null,
            activeRemoteHost: null,
            activeProjectRoot: null,
          }
        : {}),
    });
    const creds = cloudCreds(get().auth);
    if (creds) void cloudDelete(creds, id).catch(() => {});
  },

  renameConversation: async (id, title) => {
    const clean = title.trim();
    const prev = get().conversations.find((c) => c.id === id);
    if (!prev || !clean || clean === prev.title) return;
    const updated = { ...prev, title: clean, titleLocked: true };
    set({ conversations: get().conversations.map((c) => (c.id === id ? updated : c)) });
    // Persist the title to the cloud. A `put` carries the whole snapshot, so
    // fetch it (cache or cloud) first — this also covers renaming a chat that
    // wasn't opened this session.
    const creds = cloudCreds(get().auth);
    if (!creds) return;
    const snap = await fetchSnapshot(id, get().auth);
    if (snap) scheduleCloudPut(creds, updated, snap);
  },

  toggleConversationSelection: (id) =>
    set((s) => {
      const next = new Set(s.selectedConversationIds);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return { selectedConversationIds: next };
    }),

  setConversationSelection: (ids) => set({ selectedConversationIds: new Set(ids) }),

  archiveSelectedConversations: () => {
    const ids = [...get().selectedConversationIds];
    if (ids.length === 0) return;
    // Skip any that are mid-run — archiving tears down the live session.
    const busy = ids.filter((id) => {
      const entry = liveSessions.get(id);
      return entry && isBusy(entry.live);
    });
    if (busy.length > 0) get().flashNotice(BUSY_SESSION_MESSAGE);
    const targets = ids.filter((id) => !busy.includes(id));
    if (targets.length === 0) return;
    for (const id of targets) closeLiveSession(get().bridge, id);
    const activeCleared = targets.includes(get().session?.id ?? "");
    set((s) => ({
      conversations: s.conversations.map((c) =>
        targets.includes(c.id) ? { ...c, archived: true } : c,
      ),
      runningIds: s.runningIds.filter((r) => !targets.includes(r)),
      selectedConversationIds: new Set(),
      ...(activeCleared
        ? {
            session: null,
            snapshot: emptySnapshot(),
            error: null,
            attachments: [],
            historyPrefix: null,
            queued: [],
            terminalOpen: false,
            activeRemote: null,
            activeRemoteHost: null,
            activeProjectRoot: null,
          }
        : {}),
    }));
    const creds = cloudCreds(get().auth);
    if (creds) for (const id of targets) void cloudSetArchived(creds, id, true).catch(() => {});
  },

  deleteSelectedConversations: () => {
    const ids = [...get().selectedConversationIds];
    if (ids.length === 0) return;
    const busy = ids.filter((id) => {
      const entry = liveSessions.get(id);
      return entry && isBusy(entry.live);
    });
    if (busy.length > 0) get().flashNotice(BUSY_SESSION_MESSAGE);
    const targets = ids.filter((id) => !busy.includes(id));
    if (targets.length === 0) return;
    for (const id of targets) {
      closeLiveSession(get().bridge, id);
      snapshotCache.delete(id);
    }
    const activeCleared = targets.includes(get().session?.id ?? "");
    set((s) => ({
      conversations: s.conversations.filter((c) => !targets.includes(c.id)),
      runningIds: s.runningIds.filter((r) => !targets.includes(r)),
      selectedConversationIds: new Set(),
      ...(activeCleared
        ? {
            session: null,
            snapshot: emptySnapshot(),
            error: null,
            attachments: [],
            historyPrefix: null,
            queued: [],
            terminalOpen: false,
            activeRemote: null,
            activeRemoteHost: null,
            activeProjectRoot: null,
          }
        : {}),
    }));
    const creds = cloudCreds(get().auth);
    if (creds) for (const id of targets) void cloudDelete(creds, id).catch(() => {});
  },

  updateModelSettings: async ({ model, reasoningEffort }) => {
    const { session, chatModels, localSettings } = get();
    const id = session?.id;

    // Resolve the new effective values for the target. When a chat is open,
    // the change scopes to THAT chat: start from its current effective values
    // (or the global default) and apply the patch. Other chats are untouched.
    // With no open chat (the start screen), edit the global default instead.
    let nextChatModels = chatModels;
    let effectiveModel: string;
    let effectiveEffort: string;
    let nextLocal = localSettings;

    if (id) {
      const ov = chatModels[id] ?? {
        model: localSettings.model,
        reasoningEffort: localSettings.reasoningEffort,
      };
      effectiveModel = model !== undefined ? model : ov.model;
      effectiveEffort = normalizeReasoningEffort(
        effectiveModel,
        reasoningEffort !== undefined ? reasoningEffort : ov.reasoningEffort,
      );
      nextChatModels = {
        ...chatModels,
        [id]: { model: effectiveModel, reasoningEffort: effectiveEffort },
      };
      saveChatModels(nextChatModels);
      set({ chatModels: nextChatModels });
    } else {
      const nextModel = model !== undefined ? model : localSettings.model;
      nextLocal = {
        ...localSettings,
        model: nextModel,
        reasoningEffort: normalizeReasoningEffort(
          nextModel,
          reasoningEffort !== undefined ? reasoningEffort : localSettings.reasoningEffort,
        ),
      };
      effectiveModel = nextLocal.model;
      effectiveEffort = nextLocal.reasoningEffort;
      saveLocalSettings(nextLocal);
      set({ localSettings: nextLocal });
    }

    // Hot-swap the live provider's LLM. `reconfigure` re-runs connect on the
    // EXISTING instance, so the model-visible transcript survives and the next
    // turn continues with full context on the new model.
    const { bridge, activeRemote } = get();
    if (!bridge?.reconfigure || !session || session.provider !== "local") return;
    // Build the connect config from the chat's EFFECTIVE settings so a remote
    // root + per-chat model both survive the hot-swap.
    const effSettings = { ...nextLocal, model: effectiveModel, reasoningEffort: effectiveEffort };
    try {
      const config = activeRemote
        ? localConnectConfig(effSettings, remoteTarget(activeRemote))
        : localConnectConfig(effSettings);
      await bridge.reconfigure(session.id, config);
    } catch (e) {
      set({ error: `Model switch failed: ${String(e)}` });
    }
  },

  setComposerPrefill: (text, timelineIndex) =>
    set({ composerPrefill: text === null ? null : { text, timelineIndex } }),

  shareConversation: async () => {
    const { session, auth } = get();
    const id = session?.id;
    const creds = cloudCreds(auth);
    if (!id) return;
    if (!creds) {
      set({ error: "Sign in to share — links serve the cloud copy of the conversation." });
      return;
    }
    try {
      const url = await cloudShare(creds, id);
      const copied = await copyText(url);
      get().flashNotice(
        copied ? "Share link copied — anyone with it can view this chat." : "Sharing on — link ready to copy.",
      );
      void notify("Share link copied", "Anyone with the link can view this conversation.");
    } catch (e) {
      set({ error: `Sharing failed: ${String(e)}` });
    }
  },

  unshareConversation: async () => {
    const { session, auth } = get();
    const id = session?.id;
    const creds = cloudCreds(auth);
    if (!id || !creds) return;
    try {
      await cloudUnshare(creds, id);
      get().flashNotice("Sharing stopped — the public link no longer works.");
      void notify("Sharing stopped", "The public link no longer works.");
    } catch (e) {
      set({ error: `Stopping the share failed: ${String(e)}` });
    }
  },

  addFiles: async (files) => {
    const incoming = files.filter((f) => f.size <= MAX_ATTACHMENT_BYTES);
    const tooBig = files.length - incoming.length;
    if (tooBig > 0) {
      set({ error: `${tooBig} file(s) skipped — over ${MAX_ATTACHMENT_BYTES / 1024 / 1024}MB.` });
    }
    const prepared = await Promise.all(incoming.map(fileToAttachment));
    set((s) => ({ attachments: [...s.attachments, ...prepared] }));
  },

  removeAttachment: (id) => {
    const a = get().attachments.find((x) => x.id === id);
    if (a?.previewUrl) URL.revokeObjectURL(a.previewUrl);
    set((s) => ({ attachments: s.attachments.filter((x) => x.id !== id) }));
  },

  resendFrom: async (timelineIndex, text) => {
    let state = get();
    const { bridge, session, snapshot } = state;
    if (!bridge || !session) return;
    if (state.updateWaiting || state.updateApplying) {
      get().flashNotice("Clark Code is finishing active work before updating; edit after it relaunches.");
      return;
    }
    const rejectEdit = (error: string) =>
      set({ error, composerPrefill: { text, timelineIndex } });
    if (session.provider !== "local") {
      rejectEdit("Editing earlier turns is currently available in Clark Code only.");
      return;
    }
    if (!text.trim() && state.attachments.length === 0) return;
    if (isBusy(snapshot)) {
      rejectEdit("Stop the current run before editing an earlier message.");
      return;
    }
    const target = snapshot.timeline[timelineIndex];
    if (target?.item !== "message" || target.role !== "user") {
      rejectEdit("That message changed before it could be edited. Try again.");
      return;
    }
    const previousEntry = liveSessions.get(session.id);
    if (!previousEntry) {
      rejectEdit("This conversation is no longer live. Reopen it and try again.");
      return;
    }

    await get().ensureCodeKey();
    state = get();
    const projectRoot = previousEntry.projectRoot || state.activeProjectRoot;
    if (!projectRoot) {
      rejectEdit("This conversation has no project folder to resume from.");
      return;
    }

    const prefix = snapshotBeforeTimelineItem(snapshot, timelineIndex);
    const historyPrefix = prefix.timeline.length > 0 ? prefix : null;
    const resume = buildResumeTranscript(prefix);
    const effective = effectiveModelSettings(state.localSettings, state.chatModels, session.id);
    const settings = { ...state.localSettings, ...effective, cwd: projectRoot };
    const config = previousEntry.remote
      ? localConnectConfig(settings, remoteTarget(previousEntry.remote))
      : localConnectConfig(settings);
    const options: SessionOptions = {
      cwd: projectRoot,
      mode: state.approvalPolicy,
      collaboration_mode: state.collaborationMode,
      ...(resume ? { resume } : {}),
    };
    const uploads = state.attachments.map(toUpload);
    const previousMeta = state.conversations.find((conversation) => conversation.id === session.id);
    const meta: ConversationMeta = previousMeta ?? {
      id: session.id,
      title: deriveTitle(snapshot),
      provider: "local",
      project: projectRoot,
      remoteHost: previousEntry.remoteHost ?? undefined,
      mode: session.mode,
      createdAt: Date.now(),
      updatedAt: Date.now(),
    };

    let detached = false;
    let replaced = false;
    let ready = false;
    let opened: Session | null = null;
    set({ connecting: true, error: null });
    try {
      await bridge.connect("local", config);
      // Ignore the clean snapshot emitted while the replacement session is
      // registered. Otherwise it can be routed into the old live entry and
      // briefly restore the abandoned branch before this function swaps it.
      liveSessions.delete(session.id);
      detached = true;
      opened = await bridge.newSession("local", options, session.id);
      replaced = true;
      await bindCloudTrajectory(bridge, opened, meta, state.auth, {
        resumed: true,
        editedFromTimelineIndex: timelineIndex,
        projectMode: previousEntry.remote ? "remote" : "local",
        model: effective.model,
        reasoningEffort: effective.reasoningEffort,
        approvalPolicy: state.approvalPolicy,
        outputStyle: state.outputStyle,
      });

      const nextEntry = newLiveEntry(opened, {
        historyPrefix,
        remote: previousEntry.remote,
        remoteHost: previousEntry.remoteHost,
        projectRoot,
      });
      liveSessions.set(session.id, nextEntry);
      snapshotCache.set(session.id, prefix);
      resetFanOut();
      for (const attachment of state.attachments) {
        if (attachment.previewUrl) URL.revokeObjectURL(attachment.previewUrl);
      }
      set({
        session: opened,
        snapshot: prefix,
        historyPrefix,
        attachments: [],
        queued: [],
        connecting: false,
        runningIds: get().runningIds.filter((id) => id !== session.id),
        dismissedFailedRuns: [],
      });

      const creds = cloudCreds(state.auth);
      if (creds) scheduleCloudPut(creds, meta, prefix, "idle");
      ready = true;
      nextEntry.starting = true;
      try {
        await bridge.prompt(session.id, [{ type: "text", text }], uploads);
      } finally {
        nextEntry.starting = false;
      }
    } catch (error) {
      if (ready) {
        set({ error: String(error), connecting: false });
        return;
      }
      if (detached && !replaced) liveSessions.set(session.id, previousEntry);
      if (replaced) {
        liveSessions.delete(session.id);
        if (opened) void bridge.closeSession?.(opened.id);
        if (previousEntry.remote) void sshDisconnect(previousEntry.remote.id);
        set({
          session: null,
          snapshot: emptySnapshot(),
          historyPrefix: null,
          activeRemote: null,
          activeRemoteHost: null,
          activeProjectRoot: null,
        });
      }
      set({
        error: String(error),
        connecting: false,
        composerPrefill: { text, timelineIndex },
      });
    }
  },

  send: async (text) => {
    const { bridge, session, attachments, snapshot } = get();
    if (!bridge || !session) return;
    if (get().updateWaiting || get().updateApplying) {
      get().flashNotice("Clark Code is finishing active work before updating; send after it relaunches.");
      return;
    }
    if (!text.trim() && attachments.length === 0) return;
    const uploads = attachments.map(toUpload);
    for (const a of attachments) if (a.previewUrl) URL.revokeObjectURL(a.previewUrl);
    set({ attachments: [], error: null });
    // A run is active in THIS conversation: queue by default. The queue drains
    // in order after each run settles, so a follow-up never changes the work
    // already in progress unless the user explicitly chooses "Steer" on it.
    if (isBusy(snapshot)) {
      const queuedMessage = { id: crypto.randomUUID(), text, uploads };
      const entry = liveSessions.get(session.id);
      if (entry) entry.queued = [...entry.queued, queuedMessage];
      set((s) => ({ queued: [...s.queued, queuedMessage] }));
      return;
    }
    try {
      const entry = liveSessions.get(session.id);
      if (entry) entry.starting = true;
      try {
        await bridge.prompt(session.id, [{ type: "text", text }], uploads);
      } finally {
        if (entry) entry.starting = false;
      }
    } catch (e) {
      // Surface the failure instead of silently doing nothing.
      set({ error: String(e) });
    }
  },

  steerQueued: async (id) => {
    const { bridge, session, queued, snapshot } = get();
    const message = queued.find((candidate) => candidate.id === id);
    if (!bridge?.steer || !session || session.provider !== "local" || !message) return;
    if (message.uploads.length > 0) {
      get().flashNotice("Messages with attachments stay queued until Clark finishes.");
      return;
    }
    if (!isBusy(snapshot)) return;
    try {
      await bridge.steer(session.id, [{ type: "text", text: message.text }]);
      get().removeQueued(id);
    } catch {
      // The run may have settled between the click and the native command. Keep
      // the message safely queued; the normal drain will send it next.
      get().flashNotice("Clark finished before the message could steer; it remains queued.");
    }
  },

  removeQueued: (id) => {
    const session = get().session;
    const entry = session ? liveSessions.get(session.id) : undefined;
    if (entry) entry.queued = entry.queued.filter((q) => q.id !== id);
    set((s) => ({ queued: s.queued.filter((q) => q.id !== id) }));
  },

  setApprovalPolicy: (mode) => {
    saveApprovalPolicy(mode);
    const { bridge, session } = get();
    const localSessionIds = new Set<string>();
    for (const entry of liveSessions.values()) {
      if (entry.session.provider !== "local") continue;
      entry.session = { ...entry.session, mode };
      localSessionIds.add(entry.session.id);
    }
    if (session?.provider === "local") localSessionIds.add(session.id);
    set({
      approvalPolicy: mode,
      ...(session?.provider === "local" ? { session: { ...session, mode } } : {}),
    });
    if (bridge?.setMode) {
      for (const id of localSessionIds) {
        void bridge.setMode(id, mode).catch((error) => set({ error: String(error) }));
      }
    }
    const { snapshot } = get();
    // If a prompt is open and the new mode would grant it, resolve it now.
    const pend = snapshot.pending_permission;
    if (bridge && session && pend && wouldAutoApprove(mode, pend)) {
      const opt = pickAllowOption(pend);
      if (opt) {
        void bridge
          .respond(session.id, { kind: "permission", request: pend.id, option: opt.id })
          .catch((e) => set({ error: String(e) }));
      }
    }
  },

  cycleApprovalPolicy: () => {
    const { approvalPolicy, setApprovalPolicy, session, activeProvider } = get();
    // Permission modes only govern the local engine; with a cloud session (or
    // a cloud target on the start screen) the pill is hidden and Shift+Tab
    // cycling an invisible mode would just surprise the next local session.
    const isLocalTarget = session ? session.provider === "local" : activeProvider === "local";
    if (!isLocalTarget) return;
    setApprovalPolicy(nextApprovalPolicy(approvalPolicy));
  },

  setCollaborationMode: (mode) => {
    saveCollaborationMode(mode);
    const { bridge, session } = get();
    set({
      collaborationMode: mode,
      ...(session ? { session: { ...session, collaboration_mode: mode } } : {}),
    });
    if (bridge?.setCollaborationMode && session) {
      void bridge.setCollaborationMode(session.id, mode).catch((error) => {
        set({ error: String(error) });
      });
    }
  },

  decidePlan: async (planId, decision) => {
    const { bridge, session } = get();
    if (!bridge || !session) return;
    try {
      await bridge.respond(session.id, {
        kind: "plan_decision",
        plan_id: planId,
        decision,
      });
      if (decision.action === "implement") {
        saveCollaborationMode("default");
        set((state) => ({
          collaborationMode: "default",
          session: state.session ? { ...state.session, collaboration_mode: "default" } : null,
          snapshot: state.snapshot.proposed_plan?.id === planId
            ? {
                ...state.snapshot,
                proposed_plan: { ...state.snapshot.proposed_plan, status: "approved" },
                timeline: state.snapshot.timeline.map((item) =>
                  item.item === "proposed_plan" && item.plan.id === planId
                    ? { ...item, plan: { ...item.plan, status: "approved" } }
                    : item,
                ),
              }
            : state.snapshot,
        }));
        await get().send("Implement the approved plan.");
      } else if (decision.feedback?.trim()) {
        await get().send(decision.feedback.trim());
      }
    } catch (error) {
      set({ error: String(error) });
      throw error;
    }
  },

  setOutputStyle: (style) => {
    saveOutputStyle(style);
    set({ outputStyle: style });
    const { bridge, session } = get();
    if (bridge?.setOutputStyle && session) {
      void bridge.setOutputStyle(session.id, style).catch(() => {});
    }
  },

  toggleTerminal: () => set((s) => ({
    terminalOpen: s.activeRemote ? false : !s.terminalOpen,
  })),
  setTerminalOpen: (open) => set({ terminalOpen: open }),
  openProjectTerminal: async (path) => {
    let target = path?.trim();
    if (!target) {
      try {
        target = (await pickFolder(get().localSettings.cwd || undefined))?.trim() || undefined;
      } catch (e) {
        set({ error: String(e) });
        return;
      }
    }
    if (!target) return;
    const cwd = target;
    get().setProjectFolder(cwd);
    set((s) => ({
      terminalOpen: true,
      terminalLaunch: { cwd, nonce: (s.terminalLaunch?.nonce ?? 0) + 1 },
    }));
  },
  setMcpOpen: (open) => set({ mcpOpen: open }),
  setSshOpen: (open) => set({ sshOpen: open }),
  setSettingsOpen: (open, section) =>
    set({ settingsOpen: open, ...(section ? { settingsSection: section } : {}) }),
  setPaletteOpen: (open) => set({ paletteOpen: open }),
  togglePalette: () => set((s) => ({ paletteOpen: !s.paletteOpen })),
  toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
  setSidebarCollapsed: (collapsed) => set({ sidebarCollapsed: collapsed }),

  cancelActive: async () => {
    const { bridge, session, snapshot } = get();
    if (!bridge || !session) return;
    const runId = Object.values(snapshot.runs).find(
      (r) => r.status === "running" || r.status === "queued",
    )?.id;
    if (runId) await bridge.cancel(session.id, runId);
  },

  resolvePermission: async (option) => {
    const { bridge, session, snapshot } = get();
    if (!bridge || !session || !snapshot.pending_permission) return;
    const response: ClientResponse = {
      kind: "permission",
      request: snapshot.pending_permission.id,
      option,
    };
    try {
      await bridge.respond(session.id, response);
    } catch (e) {
      // Without this the click silently does nothing and the gate just sits
      // there — surface it like every other failed action.
      set({ error: String(e) });
      throw e; // let the gate re-enable its buttons
    }
  },

  askSideQuestion: async (question) => {
    const text = question.trim();
    if (!text) return;
    const { bridge, session } = get();
    if (!bridge || !session) return;
    // The fork lives on the host; if this bridge can't fork, surface it in the
    // overlay rather than as a generic app error.
    if (!bridge.sideQuestion) {
      set({ sideQuestion: { question: text, answer: null, error: "Side questions aren't available here.", loading: false, token: 0 } });
      return;
    }
    const token = (get().sideQuestion?.token ?? 0) + 1;
    set({ sideQuestion: { question: text, answer: null, error: null, loading: true, token } });
    try {
      const answer = await bridge.sideQuestion(session.id, text);
      // Drop a stale result: the user dismissed or asked a newer question.
      const current = get().sideQuestion;
      if (!current || current.token !== token) return;
      set({ sideQuestion: { ...current, answer, error: null, loading: false } });
    } catch (e) {
      const current = get().sideQuestion;
      if (!current || current.token !== token) return;
      set({ sideQuestion: { ...current, answer: null, error: String(e), loading: false } });
    }
  },

  dismissSideQuestion: () => {
    // Bump the token so an in-flight answer can't revive the overlay after the
    // user closed it. Never touches the main run's cancellation path.
    const current = get().sideQuestion;
    set({ sideQuestion: null });
    if (current) void current; // token dies with the cleared state
  },

}));

// Dev-only test seam: lets headless harnesses inject store state (e.g. a low
// credit balance) to exercise UI that depends on the live backend. Stripped from
// production builds.
if (import.meta.env.DEV && typeof window !== "undefined") {
  (window as unknown as { __clarkStore?: typeof useSessionStore }).__clarkStore =
    useSessionStore;
}
