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
  renderResumeContext,
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
  localSettingsReady,
  type LocalAgentSettings,
} from "../lib/localAgent";
import { pickFolder } from "../lib/pickFolder";
import { sshConnect, sshDisconnect, remoteTarget, type RemoteInfo } from "../lib/ssh";
import { loadSshHosts, hostReady, type SshHost } from "../lib/sshHosts";
import {
  loadPermissionMode,
  savePermissionMode,
  pickAllowOption,
  wouldAutoApprove,
  nextPermissionMode,
  type PermissionMode,
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
} from "../lib/cloudHistory";
import { provisionCodeKey, billingMe, type BillingSummary } from "../lib/account";
import { copyText } from "../lib/clipboard";
import { notify } from "../lib/notify";
import { repositoryFingerprintForRoot } from "../lib/repositoryKnowledge";
import {
  checkAndStageUpdate,
  relaunchApp,
  consumeJustUpdated,
  type StagedUpdate,
  type DownloadProgress,
} from "../lib/updater";

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
    plan: live.plan ?? prefix.plan,
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
  /** Run ids whose "Run failed" banner the user has dismissed this session. */
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
  /** Text staged into the composer by "Edit & resend" on a sent message. */
  composerPrefill: string | null;
  /** Config for the "Local coding" provider (persisted to localStorage). */
  localSettings: LocalAgentSettings;
  /** Where the next session runs: this machine, or a remote host over SSH. */
  projectMode: "local" | "remote";
  /** The saved SSH host selected for a remote session (id into sshHosts). */
  selectedHostId: string | null;
  /** The live remote connection for the active session (null when local). Held
   *  for teardown (ssh_disconnect) and to tag the conversation as remote. */
  activeRemote: RemoteInfo | null;
  /** The SSH destination of the active remote session, for the history badge. */
  activeRemoteHost: string | null;
  /** Whether durable memory is enabled (global user preference; the agent gets
   *  the `memory` tool and its saved facts are injected into the prompt). */
  memoriesEnabled: boolean;
  /** Whether the experimental `browser` tool is enabled (off by default —
   *  downloads clark-browser, ~150-300MB, on first use). */
  browserEnabled: boolean;
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
  permissionMode: PermissionMode;
  /** The agent's reply tone/persona for this session — see `lib/outputStyle.ts`. */
  outputStyle: string;
  /** Whether the in-chat terminal drawer is open. */
  terminalOpen: boolean;
  /** Whether the MCP servers settings modal is open. */
  mcpOpen: boolean;
  /** Whether the remote-hosts (SSH) settings modal is open. */
  sshOpen: boolean;
  /** Whether the unified Settings modal is open, and which section it shows. */
  settingsOpen: boolean;
  settingsSection: SettingsSection;
  /** Whether the ⌘K command palette is open. */
  paletteOpen: boolean;
  /** Whether the sidebar is collapsed to its icon rail. */
  sidebarCollapsed: boolean;
  /** Billing summary (plan, subscription, credits) from Clark; null until loaded. */
  billing: BillingSummary | null;
  loadingBilling: boolean;
  /** A downloaded + staged app update awaiting a relaunch to apply. */
  update: StagedUpdate | null;
  /** Live byte progress while an update downloads in the background; null when idle. */
  updateProgress: DownloadProgress | null;
  /** True from "Restart to update" being clicked until the relaunch takes. */
  updateApplying: boolean;
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
  /** Change the coding model / reasoning effort. Persists, and when a session is
   *  live, hot-swaps the provider's LLM (the transcript is kept — the next turn
   *  continues with full context on the new model). */
  updateModelSettings: (patch: { model?: string; reasoningEffort?: string }) => Promise<void>;
  /** Stage text in the composer ("Edit & resend" on a sent message). */
  setComposerPrefill: (text: string | null) => void;
  /** Create a public read-only link for the viewed conversation + copy it. */
  shareConversation: () => Promise<void>;
  /** Revoke the viewed conversation's public link. */
  unshareConversation: () => Promise<void>;
  addFiles: (files: File[]) => Promise<void>;
  removeAttachment: (id: string) => void;
  send: (text: string) => Promise<void>;
  removeQueued: (id: string) => void;
  setPermissionMode: (mode: PermissionMode) => void;
  /** Shift+Tab: advance to the next permission mode in the cycle. */
  cyclePermissionMode: () => void;
  setOutputStyle: (style: string) => void;
  toggleTerminal: () => void;
  setTerminalOpen: (open: boolean) => void;
  setMcpOpen: (open: boolean) => void;
  setSshOpen: (open: boolean) => void;
  /** Open/close the unified Settings modal, optionally jumping to a section. */
  setSettingsOpen: (open: boolean, section?: SettingsSection) => void;
  setPaletteOpen: (open: boolean) => void;
  togglePalette: () => void;
  /** Check for, download, verify, and stage a newer version (no-op outside the app). */
  checkForUpdate: () => Promise<void>;
  /** Relaunch into the staged update. */
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
  /** Hide the "Run failed" banner for a specific run. */
  dismissFailedRun: (runId: string) => void;
  toggleSidebar: () => void;
  setSidebarCollapsed: (collapsed: boolean) => void;
  cancelActive: () => Promise<void>;
  resolvePermission: (option: string) => Promise<void>;
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
  /** Project folder captured at open time (local sessions), so background
   *  persistence doesn't misattribute the project when settings change. */
  projectCwd: string | null;
  /** Follow-ups typed while this conversation's run was streaming. */
  queued: QueuedMessage[];
  // Per-session bookkeeping for the shared snapshot handler.
  lastPersist: number;
  prevBusy: boolean;
  dispatching: boolean;
  autoResolvedId: string | null;
  notifiedPermId: string | null;
}

/** The pool of live sessions, keyed by conversation id. */
const liveSessions = new Map<string, LiveEntry>();

function newLiveEntry(
  session: Session,
  init: Pick<LiveEntry, "historyPrefix" | "remote" | "remoteHost" | "projectCwd">,
): LiveEntry {
  return {
    session,
    live: { ...emptySnapshot(), session: session.id },
    queued: [],
    lastPersist: 0,
    prevBusy: false,
    dispatching: false,
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
async function openRemote(host: SshHost): Promise<RemoteInfo> {
  if (!hostReady(host)) {
    throw new Error("This host needs a remote folder and an exec-server binary path.");
  }
  return sshConnect(host.host.trim(), host.remoteRoot.trim(), host.binaryPath.trim());
}

// Chat history is cloud-only. Snapshots are cached in memory for the app's
// lifetime (never persisted to disk) so re-opening a conversation within a
// session is instant; a cold start re-fetches from the cloud. The conversation
// LIST lives in the store's `conversations` and is populated from the cloud on
// init/sign-in (see `syncCloudIndex`).
const snapshotCache = new Map<string, Snapshot>();

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
  projectMode: "local",
  selectedHostId: loadSshHosts()[0]?.id ?? null,
  activeRemote: null,
  activeRemoteHost: null,
  memoriesEnabled: loadMemoriesEnabled(),
  browserEnabled: loadBrowserEnabled(),
  memoryStatus: null,
  memoryViewerOpen: false,
  loadingMemory: false,
  memoryOverview: null,
  globalMemoryOverview: null,
  recentProjects: loadRecentProjects(),
  queued: [],
  permissionMode: loadPermissionMode(),
  outputStyle: loadOutputStyle(),
  terminalOpen: false,
  mcpOpen: false,
  sshOpen: false,
  settingsOpen: false,
  settingsSection: "general",
  paletteOpen: false,
  sidebarCollapsed: false,
  billing: null,
  loadingBilling: false,
  update: null,
  updateProgress: null,
  updateApplying: false,
  justUpdatedTo: null,

  checkForUpdate: async () => {
    if (get().update || get().updateProgress) return; // already staged or in flight
    const staged = await checkAndStageUpdate((p) => set({ updateProgress: p }));
    set({ updateProgress: null, ...(staged ? { update: staged } : {}) });
  },
  applyUpdate: async () => {
    set({ updateApplying: true });
    await relaunchApp();
    // Still running means the relaunch didn't take — release the overlay so the
    // user isn't trapped behind it.
    set({ updateApplying: false });
  },
  dismissJustUpdated: () => set({ justUpdatedTo: null }),
  dismissError: () => set({ error: null }),
  flashNotice: (message) => set({ notice: message }),
  dismissNotice: () => set({ notice: null }),
  dismissWarning: () => set({ warning: null }),
  dismissFailedRun: (runId) =>
    set((s) =>
      s.dismissedFailedRuns.includes(runId)
        ? s
        : { dismissedFailedRuns: [...s.dismissedFailedRuns, runId] },
    ),

  loadBilling: async () => {
    const creds = cloudCreds(get().auth);
    if (!creds) {
      set({ billing: null });
      return;
    }
    set({ loadingBilling: true });
    try {
      const billing = await billingMe(creds);
      set({ billing, loadingBilling: false });
    } catch {
      set({ loadingBilling: false });
    }
  },

  init: async () => {
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
                  ? (entry.remoteHost ? entry.remote?.cwd : entry.projectCwd) || undefined
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
        if (entry.prevBusy && !busyNow) {
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
          if (now - lastBilling > 15000) {
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
          if (pend.id !== entry.autoResolvedId && wouldAutoApprove(get().permissionMode, pend)) {
            const opt = pickAllowOption(pend);
            if (opt) {
              entry.autoResolvedId = pend.id;
              bridge
                .respond(id, { kind: "permission", request: pend.id, option: opt.id })
                .catch((e) => set({ error: String(e) }));
            }
          } else if (pend.id !== entry.notifiedPermId && !wouldAutoApprove(get().permissionMode, pend)) {
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
      // Auto-update: check shortly after launch, then every 6h. Downloads +
      // verifies + stages in the background; the UI surfaces "Restart to update".
      setTimeout(() => void get().checkForUpdate(), 4000);
      setInterval(() => void get().checkForUpdate(), 6 * 60 * 60 * 1000);
      // If we just relaunched into a freshly-applied update, confirm it once.
      void consumeJustUpdated().then((v) => {
        if (v) set({ justUpdatedTo: v });
      });
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

  loadMemory: async () => {
    const { bridge, localSettings: s } = get();
    const cwd = s.cwd.trim();
    if (!bridge?.listMemory) {
      set({ memoryOverview: null, globalMemoryOverview: null });
      return;
    }
    set({ loadingMemory: true });
    try {
      // Project scope needs a folder; global scope is per-user (always available).
      const [memoryOverview, globalMemoryOverview] = await Promise.all([
        cwd ? bridge.listMemory(cwd) : Promise.resolve(null),
        bridge.listGlobalMemory?.() ?? Promise.resolve(null),
      ]);
      set({ loadingMemory: false, memoryOverview, globalMemoryOverview, memoryStatus: null });
    } catch (e) {
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
    set({ auth, conversations: [], conversationsLoading: true });
    // Provision the Clark Code key; migrate any residual local chats into this
    // account's cloud (one-time — self-cleaning); then pull the cloud list.
    void get().ensureCodeKey();
    get().migrateLocalToCloud();
    void get().syncCloudIndex();
  },

  signOutAuth: () => {
    authSignOut();
    get().endSession({ force: true });
    // Drop the in-memory history entirely so the signed-out (and any next)
    // account starts clean.
    snapshotCache.clear();
    set({ auth: null, billing: null, conversations: [], conversationsLoading: false });
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
      if (isRemote) {
        const host = loadSshHosts().find((h) => h.id === get().selectedHostId);
        if (!host) throw new Error("Pick a remote host first, or add one.");
        remote = await openRemote(host);
        remoteHost = host.host.trim();
        config = localConnectConfig(localSettings, remoteTarget(remote));
        options = { cwd: remote.cwd };
      } else if (isLocal) {
        config = localConnectConfig(localSettings);
        options = { cwd: localSettings.cwd.trim() };
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
      await bindCloudTrajectory(
        bridge,
        session,
        {
          id: session.id,
          title: "New conversation",
          provider: activeProvider,
          project,
          remoteHost: remoteHost ?? undefined,
          mode: session.mode,
          createdAt: Date.now(),
          updatedAt: Date.now(),
        },
        get().auth,
        {
          projectMode: get().projectMode,
          model: isLocal ? localSettings.model : undefined,
          reasoningEffort: isLocal ? localSettings.reasoningEffort : undefined,
          permissionMode: get().permissionMode,
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
          projectCwd: isLocal && !isRemote ? localSettings.cwd.trim() || null : null,
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
        activeRemote: remote,
        activeRemoteHost: remoteHost,
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
      activeRemote: null,
      activeRemoteHost: null,
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
      let config;
      let options;
      let remoteHost: string | null = null;
      if (wantRemote) {
        const host = loadSshHosts().find((h) => h.host.trim() === openingMeta!.remoteHost);
        if (!host) {
          throw new Error(`Add the SSH host "${openingMeta!.remoteHost}" to reopen this remote conversation.`);
        }
        remote = await openRemote(host);
        remoteHost = host.host.trim();
        config = localConnectConfig(localSettings, remoteTarget(remote));
        options = { cwd: remote.cwd };
      } else if (isLocal) {
        config = localConnectConfig(localSettings);
        options = { cwd: localSettings.cwd.trim() };
      } else {
        config = { endpoint: auth?.clark.endpoint, auth_token: auth?.clark.token };
        options = {};
      }

      // Providers that can't resume have no server-side context either: hand
      // the new session a rendered transcript so the MODEL remembers the prior
      // turns, not just the UI (which restores them via `historyPrefix`).
      if (!canResume && restored) {
        const resumeContext = renderResumeContext(restored);
        if (resumeContext) (options as SessionOptions).resume_context = resumeContext;
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
          ? (wantRemote ? remote?.cwd : localSettings.cwd.trim()) || undefined
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
        model: isLocal ? localSettings.model : undefined,
        reasoningEffort: isLocal ? localSettings.reasoningEffort : undefined,
        permissionMode: get().permissionMode,
        outputStyle: get().outputStyle,
      });
      liveSessions.set(
        id,
        newLiveEntry(opened, {
          historyPrefix: restored,
          remote,
          remoteHost,
          projectCwd: isLocal && !wantRemote ? localSettings.cwd.trim() || null : null,
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
          }
        : {}),
    }));
    const creds = cloudCreds(get().auth);
    if (creds) for (const id of targets) void cloudDelete(creds, id).catch(() => {});
  },

  updateModelSettings: async ({ model, reasoningEffort }) => {
    const s = get().localSettings;
    const next = {
      ...s,
      ...(model !== undefined ? { model } : {}),
      ...(reasoningEffort !== undefined ? { reasoningEffort } : {}),
    };
    saveLocalSettings(next);
    set({ localSettings: next });
    // Hot-swap the live provider's LLM. `reconfigure` re-runs connect on the
    // EXISTING instance, so the model-visible transcript survives and the next
    // turn continues with full context on the new model.
    const { bridge, session, activeRemote } = get();
    if (!bridge?.reconfigure || !session || session.provider !== "local") return;
    try {
      const config = activeRemote
        ? localConnectConfig(next, remoteTarget(activeRemote))
        : localConnectConfig(next);
      await bridge.reconfigure(session.id, config);
    } catch (e) {
      set({ error: `Model switch failed: ${String(e)}` });
    }
  },

  setComposerPrefill: (text) => set({ composerPrefill: text }),

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

  send: async (text) => {
    const { bridge, session, attachments, snapshot } = get();
    if (!bridge || !session) return;
    if (!text.trim() && attachments.length === 0) return;
    const uploads = attachments.map(toUpload);
    for (const a of attachments) if (a.previewUrl) URL.revokeObjectURL(a.previewUrl);
    set({ attachments: [], error: null });
    // A run is active in THIS conversation: queue instead of interrupting. It
    // sends automatically once the run finishes (drained in the subscribe
    // handler) — even if the user has switched to another conversation.
    if (isBusy(snapshot)) {
      const queuedMessage = { id: crypto.randomUUID(), text, uploads };
      const entry = liveSessions.get(session.id);
      if (entry) entry.queued = [...entry.queued, queuedMessage];
      set((s) => ({ queued: [...s.queued, queuedMessage] }));
      return;
    }
    try {
      await bridge.prompt(session.id, [{ type: "text", text }], uploads);
    } catch (e) {
      // Surface the failure instead of silently doing nothing.
      set({ error: String(e) });
    }
  },

  removeQueued: (id) => {
    const session = get().session;
    const entry = session ? liveSessions.get(session.id) : undefined;
    if (entry) entry.queued = entry.queued.filter((q) => q.id !== id);
    set((s) => ({ queued: s.queued.filter((q) => q.id !== id) }));
  },

  setPermissionMode: (mode) => {
    savePermissionMode(mode);
    set({ permissionMode: mode });
    // If a prompt is open and the new mode would grant it, resolve it now.
    const { bridge, session, snapshot } = get();
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

  cyclePermissionMode: () => {
    const { permissionMode, setPermissionMode, bridge, session } = get();
    const next = nextPermissionMode(permissionMode);
    setPermissionMode(next);
    // Best-effort: not every provider supports server-side mode switching
    // (e.g. an ACP agent that doesn't advertise modes) — enforcement for the
    // local agent lives in setPermissionMode's own gate-driven flow either way.
    if (bridge?.setMode && session) {
      void bridge.setMode(session.id, next).catch(() => {});
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

  toggleTerminal: () => set((s) => ({ terminalOpen: !s.terminalOpen })),
  setTerminalOpen: (open) => set({ terminalOpen: open }),
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
}));

// Dev-only test seam: lets headless harnesses inject store state (e.g. a low
// credit balance) to exercise UI that depends on the live backend. Stripped from
// production builds.
if (import.meta.env.DEV && typeof window !== "undefined") {
  (window as unknown as { __clarkStore?: typeof useSessionStore }).__clarkStore =
    useSessionStore;
}
