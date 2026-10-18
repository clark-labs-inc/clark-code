import type { SessionState } from "./sessionStore.types";
export type { SessionState, SessionSet, SessionGet } from "./sessionStore.types";
import { SnapshotPublicationCadence } from "../lib/snapshotPublicationCadence";
import {
  getBridge,
  type CloudTrajectoryConfig,
  type CoreBridge,
  type PromptReceipt,
  type ManagedWorktreeBase,
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
  refreshAuthSession,
  signInWithGoogle,
  signOut as authSignOut,
  type AuthMethod,
  type AuthSession,
} from "../lib/auth";
import {
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
  MAX_ATTACHMENT_BYTES, LOCAL_ATTACHMENT_KINDS, addRecentProject, attachmentKind, authSignOut, beginUpdateDrain,
  cancelUpdateDrain, checkAndStageUpdate, cloudCreds, cloudDelete, cloudList,
  cloudSetArchived, cloudShare, cloudUnshare, codeKeyAccountBinding, configureCloudHistoryCredentials, consumeJustUpdated,
  conversationProjectRoot, copyText, deriveTitle, effectiveModelSettings, emptySnapshot,
  fileToAttachment, getBridge, hasContent, installStagedUpdate, prepareCloudDurability,
  liveProjectRoot, loadApprovalPolicy, loadApprovalPolicies, loadBrowserEnabled, loadChatModels,
  loadCollaborationMode, loadCollaborationModes, loadLocalSettings, loadMemoriesEnabled, loadOrchestrationEnabled, loadOutputStyle, loadRecentProjects,
  loadSshHosts, localConnectConfig, localSettingsReady, minLoadDuration, nextApprovalPolicy, normalizeCodingModel, normalizeReasoningEffort,
  notify, onCloudHistoryConflict, onCloudHistoryWarning, onSettingsMenuRequested, onUpdateMenuRequested, pickAllowOption,
  pickFolder, provisionCodeKey, refreshAuthSession, refreshStagedUpdate, relaunchApp, releaseSnapshotCheckpoints, remoteTarget,
  resetCloudHistory, resetFanOut, restorePendingAttachments, revokeAttachmentPreviews, saveApprovalPolicy, saveApprovalPolicies, saveBrowserEnabled, saveChatModels, saveCollaborationMode, saveCollaborationModes,
  saveLocalSettings, saveMemoriesEnabled, saveOrchestrationEnabled, saveOutputStyle, scheduleCloudPut, settleRuns,
  signInWithGoogle, snapshotBeforeTimelineItem, syncFanOut, toUpload,
  wouldAutoApprove,
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
  /** A cloud/cached transcript is available for reading without a provider. */
  savedSnapshot?: boolean;
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
  publicationCadence: SnapshotPublicationCadence;
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
    publicationCadence: new SnapshotPublicationCadence(),
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
  const executionRoot = session.environment?.checkout_root ?? meta.project;
  const repositoryFingerprint = executionRoot
    ? await repositoryFingerprintForRoot(executionRoot, codeKeyAccountBinding(auth))
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
