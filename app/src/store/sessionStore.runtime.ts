import type { StoreApi } from "zustand";
import type { SidebarConversationMutation } from "../lib/sidebarConversationInteractions";
import {
  getBridge,
  type CloudTrajectoryConfig,
  type CoreBridge,
  type ManagedWorktreeBase,
  type PromptReceipt,
  type ProjectWorktreeTransitionPlan,
  type SessionOptions,
} from "../core-bridge/bridge";
import { syncFanOut, resetFanOut } from "./fanOutStore";
import {
  emptySnapshot,
  type ClientResponse,
  type CollaborationMode,
  type ContentBlock,
  type PlanImplementationContext,
  type ProviderInfo,
  type Session,
  type Snapshot,
  type MemoryOverview,
} from "../core-bridge/types";
import {
  attachmentKind,
  fileToAttachment,
  LOCAL_ATTACHMENT_KINDS,
  restorePendingAttachments,
  revokeAttachmentPreviews,
  toUpload,
  MAX_ATTACHMENT_BYTES,
  type PendingAttachment,
  type Upload,
} from "../lib/attachments";
import { minLoadDuration } from "../lib/minLoadDuration";
import {
  loadAuthSession,
  markAuthReconnectRequired,
  refreshAuthSession,
  signInWithGoogle,
  signOut as authSignOut,
  type AuthMethod,
  type AuthSession,
} from "../lib/auth";
import {
  buildResumeTranscript,
  migratePlanningSnapshot,
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
  normalizeCodingModel,
  normalizeReasoningEffort,
  type LocalAgentSettings,
  type ChatModelOverride,
} from "../lib/localAgent";
import { pickFolder } from "../lib/pickFolder";
import { remoteWorkerConnect, remoteTarget, type RemoteInfo } from "../lib/remoteWorker";
import { loadSshHosts, hostReady, type SshHost } from "../lib/sshHosts";
import {
  loadApprovalPolicy,
  loadApprovalPolicies,
  loadCollaborationMode,
  loadCollaborationModes,
  saveApprovalPolicy,
  saveApprovalPolicies,
  saveCollaborationMode,
  saveCollaborationModes,
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
  configureCloudHistoryCredentials,
  resetCloudHistory,
  scheduleCloudPut,
  prepareCloudDurability,
  onCloudHistoryConflict,
  onCloudHistoryWarning,
} from "../lib/cloudHistory";
import {
  provisionCodeKey,
  codeKeyAccountBinding,
} from "../lib/account";
import { copyText } from "../lib/clipboard";
import { notify } from "../lib/notify";
import { repositoryFingerprintForRoot } from "../lib/repositoryKnowledge";
import { conversationProjectRoot, liveProjectRoot } from "../lib/sessionEnvironment";
import { releaseSnapshotCheckpoints } from "../lib/checkpointRefs";
import {
  checkAndStageUpdate,
  refreshStagedUpdate,
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

export {
  MAX_ATTACHMENT_BYTES, LOCAL_ATTACHMENT_KINDS, addRecentProject, attachmentKind, authSignOut, beginUpdateDrain, buildResumeTranscript,
  cancelUpdateDrain, checkAndStageUpdate, cloudCreds, cloudDelete, cloudGet, cloudList,
  cloudSetArchived, cloudShare, cloudUnshare, codeKeyAccountBinding, configureCloudHistoryCredentials, consumeJustUpdated,
  conversationProjectRoot, copyText, deriveTitle, effectiveModelSettings, emptySnapshot,
  fileToAttachment, getBridge, hasContent, hostReady, installStagedUpdate, prepareCloudDurability,
  liveProjectRoot, loadApprovalPolicy, loadApprovalPolicies, loadAuthSession, loadBrowserEnabled, loadChatModels,
  loadCollaborationMode, loadCollaborationModes, loadLocalSettings, loadMemoriesEnabled, loadOrchestrationEnabled, loadOutputStyle, loadRecentProjects,
  loadSshHosts, localConnectConfig, localSettingsReady, minLoadDuration, nextApprovalPolicy, normalizeCodingModel, normalizeReasoningEffort,
  markAuthReconnectRequired, notify, onCloudHistoryConflict, onCloudHistoryWarning, onSettingsMenuRequested, onUpdateMenuRequested, pickAllowOption,
  pickFolder, provisionCodeKey, refreshAuthSession, refreshStagedUpdate, relaunchApp, releaseSnapshotCheckpoints, remoteTarget,
  repositoryFingerprintForRoot, resetCloudHistory, resetFanOut, restorePendingAttachments, revokeAttachmentPreviews, saveApprovalPolicy, saveApprovalPolicies, saveBrowserEnabled, saveChatModels, saveCollaborationMode, saveCollaborationModes,
  saveLocalSettings, saveMemoriesEnabled, saveOrchestrationEnabled, saveOutputStyle, scheduleCloudPut, settleRuns,
  signInWithGoogle, snapshotBeforeTimelineItem, syncFanOut, toUpload,
  updateDrainBlockerCount, wouldAutoApprove,
};
export type {
  ApprovalPolicy, AuthMethod, AuthSession, ChatModelOverride,
  ClientResponse, CloudTrajectoryConfig, CollaborationMode, ConversationMeta, CoreBridge, DownloadProgress,
  LocalAgentSettings, ManagedWorktreeBase, MemoryOverview, PendingAttachment, PlanImplementationContext,
  ProjectWorktreeTransitionPlan, ProviderInfo, RemoteInfo,
  Session, SessionOptions, Snapshot, SshHost, StagedUpdate, UpdateCheckResult,
  Upload,
  ContentBlock,
};

export type SshOpenPurpose = "manage" | "select_execution_target";

/** A follow-up message the user sent while a run was active. It sends
 *  automatically when the run finishes, never interrupting. */
/** The sections of the unified Settings view (left-rail order). */
export type SettingsSection =
  | "general"
  | "project"
  | "integrations"
  | "commands"
  | "computer-use"
  | "account"
  | "about";

export interface QueuedMessage {
  id: string;
  text: string;
  uploads: Upload[];
  skills: SkillReferenceBlock[];
}

/** Explicit result of handing a composer message to the session runtime. */
export type SendOutcome =
  | { kind: "started"; receipt: PromptReceipt }
  | { kind: "queued"; queueId: string }
  | { kind: "cancelled" }
  | { kind: "not_sent" };

export type SkillReferenceBlock = Extract<ContentBlock, { type: "skill_reference" }>;

export interface ComposerPrefill {
  text: string;
  /** Present only for edit-and-resend; identifies the turn to replace. */
  timelineIndex?: number;
}

/** A sidebar conversation that the user selected but the agent could not reopen.
 * It remains the active navigation target until retry or cleanup, rather than
 * silently snapping the workspace back to the previously rendered chat. */
export interface UnavailableConversation {
  id: string;
  title: string;
  detail: string;
  kind: "unavailable" | "refresh_required";
}

/** `/btw` overlay state. `loading` is true while the forked side-question call
 *  is in flight; exact session ownership plus a unique token lets a late
 *  result be dropped without clobbering a closed/newer overlay. */
export interface SideQuestionState {
  /** Conversation that owns this overlay and its in-flight result. */
  sessionId: string;
  question: string;
  answer: string | null;
  error: string | null;
  loading: boolean;
  /** Process-monotonic token; only a result that matches this token is applied. */
  token: number;
}

export function isBusy(snap: Snapshot): boolean {
  return Object.values(snap.runs).some(
    (r) => r.status === "running" || r.status === "queued",
  );
}

/** Completion notifications describe the run that just settled, not failures
 *  retained in the conversation's earlier history. Runs preserve insertion
 *  order from agent-core's IndexMap, matching the conversation failure banner. */
export function latestRunFailed(snap: Snapshot): boolean {
  const runs = Object.values(snap.runs);
  return runs[runs.length - 1]?.status === "failed";
}

/** A background run that just finished in a conversation the user isn't
 * looking at earns a blue "finished, not yet visited" sidebar marker. Only
 * active-screen or archived conversations are excluded; re-marking an id that
 * is already marked is a no-op so a repeat turn keeps the marker until opened. */
export function markUnseenFinished(
  current: readonly string[],
  id: string,
  activeId: string | null,
  archived: boolean,
): string[] {
  if (archived || id === activeId || current.includes(id)) return [...current];
  return [...current, id];
}

/** Visiting (opening) a conversation clears its finished-not-yet-visited marker. */
export function clearUnseenFinished(current: readonly string[], id: string): string[] {
  return current.filter((entry) => entry !== id);
}

/**
 * Old provider builds allocated run-1, run-2, ... from a process-local
 * counter. A reopened conversation therefore could reuse a run id already in
 * its restored prefix. Preserve both histories rather than letting the live
 * run overwrite the earlier terminal receipt.
 */
function rekeyCollidingLiveRuns(prefix: Snapshot, live: Snapshot): Snapshot {
  const used = new Set([...Object.keys(prefix.runs), ...Object.keys(live.runs)]);
  const aliases = new Map<string, string>();
  for (const id of Object.keys(live.runs)) {
    if (!prefix.runs[id]) continue;
    let suffix = 1;
    let alias = `${id}~resume-${suffix}`;
    while (used.has(alias)) alias = `${id}~resume-${++suffix}`;
    used.add(alias);
    aliases.set(id, alias);
  }
  if (aliases.size === 0) return live;

  const runId = (id: string): string => aliases.get(id) ?? id;
  const incidentAliases = new Map<string, string>();
  const providerIncidents = Object.fromEntries(
    Object.entries(live.provider_incidents).map(([id, incident]) => {
      let alias = id;
      for (const [oldRun, newRun] of aliases) {
        if (id === oldRun || id.startsWith(`${oldRun}:`)) {
          alias = `${newRun}${id.slice(oldRun.length)}`;
          break;
        }
      }
      if (alias !== id) incidentAliases.set(id, alias);
      return [alias, alias === id ? incident : { ...incident, id: alias }];
    }),
  );
  const runs = Object.fromEntries(
    Object.entries(live.runs).map(([id, run]) => {
      const alias = runId(id);
      const execution = run.outcome?.execution;
      const executionId = execution?.execution_id;
      const rewrittenExecution = execution && executionId?.endsWith(`:${id}`)
        ? { ...execution, execution_id: `${executionId.slice(0, -id.length)}${alias}` }
        : execution;
      return [alias, {
        ...run,
        id: alias,
        ...(run.outcome && rewrittenExecution
          ? { outcome: { ...run.outcome, execution: rewrittenExecution } }
          : {}),
      }];
    }),
  );
  const timeline = live.timeline.map((item) => {
    const withRun = "run" in item && item.run
      ? { ...item, run: runId(item.run) }
      : item;
    return withRun.item === "provider_incident"
      ? { ...withRun, id: incidentAliases.get(withRun.id) ?? withRun.id }
      : withRun;
  });
  return {
    ...live,
    runs,
    timeline,
    provider_incidents: providerIncidents,
    goal: live.goal?.run ? { ...live.goal, run: runId(live.goal.run) } : live.goal,
  };
}

/** Past transcript (prefix) + live resumed turns → one displayed snapshot. */
export function mergeHistory(prefix: Snapshot, live: Snapshot): Snapshot {
  live = rekeyCollidingLiveRuns(prefix, live);
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
    sync_pending: live.sync_pending ?? prefix.sync_pending,
    history_checkpoint: live.history_checkpoint ?? prefix.history_checkpoint,
    runs: { ...prefix.runs, ...live.runs },
    timeline: [...prefix.timeline, ...live.timeline],
    model_context_checkpoint: live.model_context_checkpoint
      ? {
          ...live.model_context_checkpoint,
          timeline_index:
            prefix.timeline.length + live.model_context_checkpoint.timeline_index,
        }
      : prefix.model_context_checkpoint,
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
    provider_incidents: {
      ...prefix.provider_incidents,
      ...live.provider_incidents,
    },
  };
}

export interface SessionState {
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
  /** Authenticated user + the agent connection config it carries. */
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
  /** Conversation ids whose background run finished while the user was in
   *  another conversation — a blue "finished, not yet visited" marker in the
   *  sidebar until the user opens it. In-memory only; cleared on visit or when
   *  the conversation is archived/deleted. */
  unseenWorkIds: string[];
  /** Conversation ids selected in the sidebar (Shift-click). Drives the
   *  right-click bulk actions (archive / delete all selected). A fresh Set
   *  on every mutation so zustand re-renders. */
  selectedConversationIds: Set<string>;
  /** Conversations currently awaiting a durable archive, delete, or restore
   * acknowledgement. Rows stay visible and explicitly busy until it arrives. */
  mutatingConversationIds: Set<string>;
  /** Sidebar bulk/single-item operation progress, kept briefly after completion
   * so visual and screen-reader feedback never disappears abruptly. */
  conversationMutation: SidebarConversationMutation | null;
  /** A session attach is in flight — drives the sidebar row spinner and, when
   *  no cached transcript exists, the opening screen. `kind` picks the copy
   *  ("Connecting" for a new session, "Reconnecting"/"Opening" for a reopen).
   *  Cleared when the session is live, the connect fails, or the user cancels
   *  (endSession). */
  opening: {
    id: string | null;
    kind: "start" | "open";
    title: string;
    remoteHost: string | null;
  } | null;
  /** The selected conversation when reopening failed. The workspace renders a
   * dedicated recovery surface while the matching sidebar row stays active. */
  unavailableConversation: UnavailableConversation | null;
  /** The unavailable entry being removed from its dedicated recovery surface.
   * Keeps that surface mounted until the full cleanup reset is ready to commit. */
  unavailableCleanupId: string | null;
  /** Text staged into the composer, optionally tied to a sent turn to replace. */
  composerPrefill: ComposerPrefill | null;
  /** Config for the "Local coding" provider (persisted to localStorage). */
  localSettings: LocalAgentSettings;
  /** Preferred immutable base for newly created managed worktrees. */
  managedWorktreeBase: ManagedWorktreeBase;
  /** A clean, isolated worktree awaiting the user's deliberate branch choice. */
  worktreeTransition: ProjectWorktreeTransitionPlan | null;
  /** A managed checkout already created for a start that can be retried. */
  pendingManagedWorktreePath: string | null;
  /** True while the host prepares a managed checkout for the next session. */
  worktreePreparing: boolean;
  /** Per-conversation model + reasoning-effort settings, keyed by conversation
   *  id and pinned when the chat is created or first reopened. Legacy chats
   *  fall back to `localSettings` only until that first open. Persisted to
   *  localStorage (the cloud stores transcripts, not model prefs). */
  chatModels: Record<string, ChatModelOverride>;
  /** Per-conversation approval-policy overrides, keyed by conversation id. A
   *  chat with no entry falls back to the account's global `approvalPolicy`
   *  default. Mirrors `chatModels`: each chat keeps its own level so cycling
   *  approval in one conversation never edits what another runs. Persisted to
   *  localStorage (the cloud stores transcripts, not prefs). */
  approvalPolicies: Record<string, ApprovalPolicy>;
  /** Where the next session runs: this machine, or a remote host over SSH. */
  projectMode: "local" | "remote";
  /** The saved SSH host selected for a remote session (id into sshHosts). */
  selectedHostId: string | null;
  /** Bumped whenever the persisted SSH host list changes (remote folder picked,
   *  host saved/edited) so UI reading `startBlockedReason()` — which re-reads
   *  localStorage — re-evaluates its blocked state. */
  sshHostsRevision: number;
  /** The native worker attachment for the active session (null when local). */
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
   *  downloads managed browser, ~150-300MB, on first use). */
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
  /** How agent permission requests are approved. */
  approvalPolicy: ApprovalPolicy;
  /** Read-only planning is independent from action approval policy. */
  collaborationMode: CollaborationMode;
  /** Per-conversation collaboration-mode overrides, keyed by conversation id.
   *  A chat with no entry falls back to the account's global `collaborationMode`
   *  default. Mirrors `approvalPolicies`: each chat keeps its own mode so
   *  switching plan mode in one conversation never edits what another runs.
   *  Persisted to localStorage (the cloud stores transcripts, not prefs). */
  collaborationModes: Record<string, CollaborationMode>;
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
  /** Why the SSH modal was opened. Adding a host from the execution picker
   *  must return that host to the picker; Settings only manages presets. */
  sshOpenPurpose: SshOpenPurpose;
  /** Whether the "New project" chooser modal is open. */
  newProjectOpen: boolean;
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
  selectProvider: (id: string) => void;
  setLocalSettings: (patch: Partial<LocalAgentSettings>) => void;
  setManagedWorktreeBase: (base: ManagedWorktreeBase) => void;
  setProjectMode: (mode: "local" | "remote") => void;
  setSelectedHostId: (id: string | null) => void;
  /** Signal that the persisted SSH host list changed (a remote folder was set
   *  or a host was saved), so `startBlockedReason` re-evaluates. */
  bumpSshHostsRevision: () => void;
  setProjectFolder: (path: string) => void;
  /** Open the native folder picker and return the selected path after it has
   *  been committed to account-scoped project state. */
  pickProjectFolder: () => Promise<string | null>;
  setMemoriesEnabled: (on: boolean) => void;
  setBrowserEnabled: (on: boolean) => void;
  setOrchestrationEnabled: (on: boolean) => void;
  loadMemory: () => Promise<void>;
  toggleMemoryViewer: () => void;
  setMemoryViewerOpen: (open: boolean) => void;
  signIn: (method: AuthMethod) => Promise<void>;
  reconnectAuth: () => Promise<void>;
  signOutAuth: () => Promise<void>;
  /** Mint + store an Clark Code API key for the signed-in user if none yet. */
  ensureCodeKey: () => Promise<void>;
  /** Fetch the account's conversation list from the cloud (the source of truth). */
  syncCloudIndex: () => Promise<void>;
  /** Why the active environment can't start a session yet (folder unset, remote
   *  host not ready…), or null when ready. Lets the composer gate a pre-session
   *  submit with the same logic the start screen uses. */
  startBlockedReason: () => string | null;
  startSession: (options?: {
    quickChat?: { id: string; path: string };
    submittedDraft?: string;
    /** Explicit user-selected folders outside the writable checkout. */
    readRoots?: string[];
  }) => Promise<void>;
  /** Start immediately in an app-managed workspace without selecting a repo. */
  startQuickChat: () => Promise<void>;
  /** Create the explicitly approved isolated checkout, then start the chat in it. */
  confirmManagedWorktreeStart: () => Promise<void>;
  /** Leave the source checkout untouched and close the preservation decision. */
  dismissManagedWorktreeStart: () => void;
  /** Detach from the current conversation (→ welcome screen). Its live session
   *  keeps running in the background pool — reopening reattaches instantly.
   *  `force` (sign-out) tears down every live session instead. */
  endSession: (opts?: { force?: boolean }) => void;
  openConversation: (id: string) => Promise<void>;
  /** Permanently remove the selected unavailable entry, then return to a clean
   * new-chat composer with no project selected. */
  cleanupUnavailableConversation: () => Promise<void>;
  /** Soft-delete: hide from the main list but keep the transcript locally and in
   *  the cloud so it can be restored. Clears the view if it's the open chat. */
  archiveConversation: (id: string) => Promise<void>;
  /** Bring an archived conversation back into the active list. */
  restoreConversation: (id: string) => Promise<void>;
  /** Permanently delete a conversation — local cache AND the cloud copy. */
  deleteConversation: (id: string) => Promise<void>;
  /** Rename a conversation; the manual title stops auto-derivation clobbering it. */
  renameConversation: (id: string, title: string) => void;
  /** Toggle one conversation in the sidebar's Shift-click selection. */
  toggleConversationSelection: (id: string) => void;
  /** Set the sidebar selection (replace). Pass an empty Set to clear. */
  setConversationSelection: (ids: Set<string>) => void;
  /** Archive every selected conversation at once. Busy ones are skipped (a
   *  notice is flashed); the rest are closed + flagged archived in the cloud. */
  archiveSelectedConversations: () => Promise<void>;
  /** Permanently delete every selected conversation at once. Busy ones are
   *  skipped (a notice is flashed); the rest are closed + deleted from the
   *  cloud. Selection is cleared afterwards. */
  deleteSelectedConversations: () => Promise<void>;
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
  send: (
    text: string,
    skills?: SkillReferenceBlock[],
  ) => Promise<SendOutcome>;
  /** Summarize and replace Clark Code's model-visible history without adding a user turn. */
  compactConversation: () => Promise<void>;
  continueProviderIncident: (incidentId: string) => Promise<void>;
  /** Replace one prior Clark Code user turn and rerun from the retained prefix. */
  resendFrom: (
    timelineIndex: number,
    text: string,
    skills?: SkillReferenceBlock[],
  ) => Promise<import("../core-bridge/bridge").PromptReceipt | null>;
  /** Explicitly inject one queued text-only message into the active local run. */
  steerQueued: (id: string) => Promise<void>;
  removeQueued: (id: string) => void;
  setApprovalPolicy: (mode: ApprovalPolicy) => void;
  /** Change only the account-wide default — never a chat's own level or a live
   *  session's mode. Used by the Settings picker; the composer pill (and
   *  Shift+Tab) edit the focused chat via `setApprovalPolicy`. */
  setDefaultApprovalPolicy: (mode: ApprovalPolicy) => void;
  /** Shift+Tab: advance to the next permission mode in the cycle. */
  cycleApprovalPolicy: () => void;
  setCollaborationMode: (mode: CollaborationMode) => void;
  decidePlan: (
    planId: string,
    decision: { action: "implement"; context: PlanImplementationContext } |
      { action: "continue_planning"; feedback?: string },
  ) => Promise<void>;
  setOutputStyle: (style: string) => void;
  /** Remove the session's standing goal. The transcript is kept intact; the
   *  goal stops continuing and its receipt is retired. */
  clearGoal: () => Promise<void>;
  toggleTerminal: () => void;
  setTerminalOpen: (open: boolean) => void;
  /** Make a folder the current project (seeding the next session). With no
   *  `path`, asks the OS for a folder first; cancelling the picker is a
   *  no-op. Does not force the terminal open — if one is already open it
   *  records a launch so a fresh tab roots at the folder. */
  openProjectTerminal: (path?: string) => Promise<void>;
  /** Set the project (a local folder or a remote SSH host + folder) and start
   *  its first session immediately instead of waiting for a typed prompt. */
  startNewProject: (target: NewProjectTarget) => Promise<void>;
  setMcpOpen: (open: boolean) => void;
  setSshOpen: (open: boolean, purpose?: SshOpenPurpose) => void;
  /** Open/close the "New project" chooser modal. */
  setNewProjectOpen: (open: boolean) => void;
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
  /** Show a transient warning toast that auto-dismisses. */
  flashWarning: (message: string) => void;
  /** Clear the transient notice toast. */
  dismissNotice: () => void;
  /** Clear the transient warning toast. */
  dismissWarning: () => void;
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

// Session transitions (start / open / end) are async. Every transition bumps
// this epoch; a continuation that awakes to find a
// newer epoch was superseded (user cancelled, hit ⌘N, or clicked another
// conversation) and must abandon instead of clobbering the newer state. Native
// workers are account-owned and intentionally survive conversation switches.
export let sessionEpoch = 0;
export const nextSessionEpoch = () => ++sessionEpoch;
export const epochStale = (epoch: number) => epoch !== sessionEpoch;

/** Shown when archive/delete would destroy a conversation whose run is still
 *  streaming. Switching chats is always safe — sessions keep running in the
 *  background — but archive/delete tears the session down for real. */
export const BUSY_SESSION_MESSAGE =
  "the agent is still working in this chat — stop the run first (⌘.), then archive or delete it.";

/** Destination for a brand-new project's first session: run it on this machine
 *  (a local folder) or over SSH on a remote host (a remote folder). */
export type NewProjectTarget =
  | { kind: "local"; path: string; base: ManagedWorktreeBase }
  | { kind: "remote"; host: SshHost };

/** One live session in the pool. Every opened conversation gets an entry that
 *  keeps its provider session (and any streaming run) alive independently of
 *  which conversation is displayed — there is no limit on how many run at
 *  once. Non-reactive on purpose: the UI renders only the ACTIVE session's
 *  snapshot (mirrored into the store) plus the lightweight `runningIds` list. */
export interface LiveEntry {
  session: Session;
  /** Latest raw engine snapshot for this session (no history prefix). */
  live: Snapshot;
  /** Restored transcript this session was reopened on top of, if any. */
  historyPrefix: Snapshot | null;
  /** Opaque attachment to the account-owned remote worker. */
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
  /** A live provider reconfiguration is in flight. Prompts must wait until
   *  the provider has finished swapping its model and tool registry. */
  reconfiguring: boolean;
  /** Last text accepted for this conversation's prompt boundary. Used only to
   *  suppress accidental duplicate clicks in a short window. */
  lastSubmittedText: string | null;
  lastSubmittedAt: number;
  autoResolvedId: string | null;
  notifiedPermId: string | null;
}

/** The pool of live sessions, keyed by conversation id. */
export const liveSessions = new Map<string, LiveEntry>();

/** Restore a follow-up whose prompt admission failed. The id guard makes a
 * late duplicate rejection harmless while preserving FIFO order. */
export function restoreQueuedAfterDispatchFailure(
  entry: LiveEntry,
  message: QueuedMessage,
): void {
  if (entry.queued.some((candidate) => candidate.id === message.id)) return;
  entry.queued = [message, ...entry.queued];
}

/** Freeze the model a local conversation was created/reopened with. Without
 * this snapshot, chats with no explicit override keep following the mutable
 * new-chat default, so changing that default makes every such chat appear to
 * switch models retroactively. */
export function pinChatModel(
  get: () => SessionState,
  set: (partial: Partial<SessionState>) => void,
  id: string,
  settings: LocalAgentSettings,
): void {
  const current = get().chatModels;
  const model = normalizeCodingModel(settings.model);
  const reasoningEffort = normalizeReasoningEffort(model, settings.reasoningEffort);
  if (
    current[id]?.model === model
    && current[id]?.reasoningEffort === reasoningEffort
  ) return;
  const next = {
    ...current,
    [id]: {
      model,
      reasoningEffort,
    },
  };
  saveChatModels(next, codeKeyAccountBinding(get().auth));
  set({ chatModels: next });
}

/** The approval policy a conversation actually runs with: its own override
 *  when it has one, otherwise the account's global default. Null when there is
 *  no open chat — the start screen uses the global default directly. */
export function effectiveApprovalPolicy(
  globalDefault: ApprovalPolicy,
  approvalPolicies: Record<string, ApprovalPolicy>,
  id: string | null | undefined,
): ApprovalPolicy {
  if (!id) return globalDefault;
  return approvalPolicies[id] ?? globalDefault;
}

/** The collaboration mode a conversation actually runs with: its own override
 *  when it has one, otherwise the account's global default. Null when there is
 *  no open chat — the start screen uses the global default directly. */
export function effectiveCollaborationMode(
  globalDefault: CollaborationMode,
  collaborationModes: Record<string, CollaborationMode>,
  id: string | null | undefined,
): CollaborationMode {
  if (!id) return globalDefault;
  return collaborationModes[id] ?? globalDefault;
}

/** Pin a chat to its current approval policy the first time it goes live, so a
 *  later change to the global default never silently rewrites what an already-
 *  running chat executes under. Idempotent: a chat already pinned keeps its
 *  own level. */
export function pinApprovalPolicy(
  get: () => SessionState,
  set: (partial: Partial<SessionState>) => void,
  id: string,
  policy: ApprovalPolicy,
): void {
  const current = get().approvalPolicies;
  if (current[id]) return;
  const next = { ...current, [id]: policy };
  saveApprovalPolicies(next, codeKeyAccountBinding(get().auth));
  set({ approvalPolicies: next });
}

export function newLiveEntry(
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
    reconfiguring: false,
    lastSubmittedText: null,
    lastSubmittedAt: 0,
    autoResolvedId: null,
    notifiedPermId: null,
    ...init,
  };
}

/** The entry's displayable snapshot: restored history + live turns. */
export function mergedOf(entry: LiveEntry): Snapshot {
  return entry.historyPrefix ? mergeHistory(entry.historyPrefix, entry.live) : entry.live;
}

/** Close one live session for real. Durable account/project workers outlive
 * conversation attachments and are reclaimed by native account teardown. */
export function closeLiveSession(bridge: CoreBridge | null, id: string): void {
  const entry = liveSessions.get(id);
  if (!entry) return;
  liveSessions.delete(id);
  void bridge?.closeSession?.(id);
}

/** Attach to (or start once) the native account/project worker for `host`. */
export async function openRemote(
  host: SshHost,
  settings: Pick<LocalAgentSettings, "model" | "reasoningEffort">,
  projectRoot = host.remoteRoot,
): Promise<RemoteInfo> {
  if (!hostReady(host)) {
    throw new Error("This host needs an SSH destination and remote folder.");
  }
  return remoteWorkerConnect(
    host.host.trim(),
    projectRoot.trim(),
    normalizeCodingModel(settings.model),
    normalizeReasoningEffort(settings.model, settings.reasoningEffort),
  );
}

// Chat history is cloud-only. Snapshots are cached in memory for the app's
// lifetime (never persisted to disk) so re-opening a conversation within a
// session is instant; a cold start re-fetches from the cloud. The conversation
// LIST lives in the store's `conversations` and is populated from the cloud on
// init/sign-in (see `syncCloudIndex`).
export const snapshotCache = new Map<string, Snapshot>();
/** Coalesce key minting per signed-in account. A sign-out or account switch
 * can happen while the request is in flight, so callers still re-check the
 * active binding before persisting the returned secret. */
export const UPDATE_DRAIN_POLL_MS = 250;
/** A provider or native stream must never be able to hold the updater hostage
 * indefinitely. The normal path drains in a few polls; this is recovery for a
 * stale frontend/native lifecycle signal. */
export const UPDATE_DRAIN_TIMEOUT_MS = 30_000;

export function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export function liveUpdateBlockerCount(): number {
  return updateDrainBlockerCount(
    [...liveSessions.values()].map((entry) => ({
      live: entry.live,
      queuedCount: entry.queued.length,
      dispatching: entry.dispatching,
      starting: entry.starting,
      reconfiguring: entry.reconfiguring,
    })),
  );
}

export const appInitializationState = {
  updateMenuListenerInstalled: false,
  settingsMenuListenerInstalled: false,
  updateTimersInstalled: false,
  initialization: null as Promise<void> | null,
};

export async function bindCloudTrajectory(
  bridge: CoreBridge,
  session: Session,
  meta: ConversationMeta,
  auth: AuthSession | null,
  metadata: Record<string, unknown>,
  baseSnapshot: Snapshot,
): Promise<void> {
  // Browser preview/dev bridges have no native cloud sink. Production Tauri
  // always implements it and requires authenticated product cloud credentials.
  if (!bridge.configureCloudTrajectory) return;
  const creds = cloudCreds(auth);
  if (!creds) {
    throw new Error("product cloud is required to start or resume a coding session.");
  }
  const repositoryFingerprint = meta.project
    ? await repositoryFingerprintForRoot(meta.project, codeKeyAccountBinding(auth))
    : null;
  const config: CloudTrajectoryConfig = {
    title: meta.title,
    provider: meta.provider,
    project: meta.project,
    repositoryFingerprint: repositoryFingerprint ?? undefined,
    remoteHost: meta.remoteHost,
    mode: meta.mode,
    metadata: {
      ...metadata,
      ...(meta.specialist ? { specialistContext: meta.specialist } : {}),
    },
  };
  // Cloud history can contain snapshots written by the pre-checklist schema.
  // Normalize at the last boundary before Tauri deserializes Snapshot so a
  // cached/replayed legacy `{ item: "plan" }` row cannot abort session setup.
  const normalizedBaseSnapshot = migratePlanningSnapshot(baseSnapshot);
  await bridge.configureCloudTrajectory(
    session.id,
    config,
    normalizedBaseSnapshot,
    meta.rev ?? 0,
  );
}

/** Cloud-first snapshot lookup: the in-memory cache, else `cloudGet`, else null.
 * A cloud snapshot may describe work owned by another currently live desktop;
 * only native restart recovery is allowed to turn that work into a terminal
 * interruption. */
export async function fetchSnapshot(
  id: string,
  auth: AuthSession | null,
  stillCurrent: () => boolean = () => true,
): Promise<Snapshot | null> {
  if (!stillCurrent()) return null;
  const cached = snapshotCache.get(id);
  if (cached) {
    if (!stillCurrent()) return null;
    const normalized = migratePlanningSnapshot(cached);
    if (normalized !== cached) snapshotCache.set(id, normalized);
    return normalized;
  }
  const creds = cloudCreds(auth);
  if (!creds) return null;
  try {
    const cloud = await cloudGet(creds, id);
    if (cloud && stillCurrent()) {
      const normalized = migratePlanningSnapshot(cloud);
      snapshotCache.set(id, normalized);
      return normalized;
    }
  } catch {
    /* offline / backend down — caller falls back to a fresh session */
  }
  return null;
}

/** Merge a cloud conversation list over the in-memory one. Cloud entries win;
 * only rows that have never received a server revision are still local-only
 * and survive an absent cloud row. A revisioned row missing from the
 * authoritative list was deleted elsewhere and must not be resurrected. */
export function mergeConversations(
  cloud: ConversationMeta[],
  local: ConversationMeta[],
): ConversationMeta[] {
  const byId = new Map<string, ConversationMeta>();
  for (const c of local) {
    if (c.rev == null) byId.set(c.id, c);
  }
  for (const c of cloud) byId.set(c.id, c);
  return [...byId.values()];
}

export const bootAuth = loadAuthSession();

export type SessionSet = StoreApi<SessionState>["setState"];
export type SessionGet = () => SessionState;
