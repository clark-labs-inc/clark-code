import { create } from "zustand";
import { getBridge, type CoreBridge } from "../core-bridge/bridge";
import { syncFanOut } from "./fanOutStore";
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
  signInWithGoogle,
  signOut as authSignOut,
  type AuthMethod,
  type AuthSession,
} from "../lib/auth";
import {
  drainLocalHistory,
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
  /** Read-only view of ANOTHER conversation while a run streams in the live one.
   *  Opening a conversation mid-run must not tear the run down, so it becomes a
   *  peek; when the run settles the peek silently promotes to a full open. */
  peek: { id: string; snapshot: Snapshot } | null;
  /** A conversation is being (re)opened — drives the "Opening…" loading screen so
   *  the UI never looks frozen during the connect (remote reopens re-establish the
   *  SSH tunnel, which can take 10–20s). Cleared once the session is live. */
  opening: { id: string; title: string; remoteHost: string | null } | null;
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
  endSession: () => void;
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
  /** Hide the "Run failed" banner for a specific run. */
  dismissFailedRun: (runId: string) => void;
  toggleSidebar: () => void;
  setSidebarCollapsed: (collapsed: boolean) => void;
  cancelActive: () => Promise<void>;
  resolvePermission: (option: string) => Promise<void>;
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
  dismissedFailedRuns: [],
  auth: bootAuth,
  attachments: [],
  conversations: [],
  conversationsLoading: !!bootAuth,
  historyPrefix: null,
  peek: null,
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
      // Guards for the snapshot handler: `autoResolvedId` stops us re-answering
      // the same permission prompt; `dispatching` stops a second queued message
      // firing before the one we just dispatched registers as a running run.
      let autoResolvedId: string | null = null;
      let dispatching = false;
      // Native-notification edges: ping when a run finishes (busy→idle) or when a
      // gate actually blocks for the user. Tracked across snapshots.
      let prevBusy = false;
      let notifiedPermId: string | null = null;
      // The host re-emits a fully cloned snapshot on every streamed token (tens
      // per second). Two throttles keep that from melting the UI:
      //   • render — coalesce to at most one React update per animation frame;
      //   • persist — write the transcript to localStorage at most ~2×/sec while
      //     a run streams, and always once it goes idle so nothing is lost.
      const raf: (cb: () => void) => void =
        typeof requestAnimationFrame !== "undefined"
          ? (cb) => requestAnimationFrame(() => cb())
          : (cb) => void setTimeout(cb, 16);
      // Buffer the RAW live snapshot and merge with the CURRENT history prefix
      // at flush time. Merging at enqueue time raced conversation switches: a
      // session-reset (empty) emission could flush AFTER openConversation set
      // the restored snapshot and blank it. Fresh-at-flush state can't go stale,
      // and it moves merge work from per-event to per-frame.
      let pending: Snapshot | null = null;
      let rafScheduled = false;
      const flushRender = () => {
        rafScheduled = false;
        if (!pending) return;
        const live = pending;
        pending = null;
        // No session (user just ended it) → nothing to render into.
        if (!get().session) return;
        const prefix = get().historyPrefix;
        const merged = prefix ? mergeHistory(prefix, live) : live;
        // Push fan-out state into its own (deduped) store on the SAME coalesced
        // frame as the render — not per raw event — so an active swarm's tiles
        // update at most once per animation frame instead of on every telemetry
        // event (the compositing/opacity churn behind the fan-out flicker).
        syncFanOut(merged.fan_out);
        set({ snapshot: merged });
      };
      let lastPersist = 0;
      let lastBilling = 0;
      // Fold each engine snapshot into the active conversation: merge with any
      // restored history prefix, show it, and persist it so the chat survives a
      // restart and can be reopened later.
      bridge.subscribe((live) => {
        const { historyPrefix, session } = get();
        const snapshot = historyPrefix ? mergeHistory(historyPrefix, live) : live;

        // Render (and fan-out sync) are coalesced to the next animation frame in
        // flushRender; the raw live snapshot is buffered here.
        pending = live;
        if (!rafScheduled) {
          rafScheduled = true;
          raf(flushRender);
        }

        const busyNow = isBusy(snapshot);

        // Persist + sidebar meta: throttled while streaming, immediate when idle.
        // While busy the interval is long (2s) and the work runs in a macrotask
        // AFTER the frame commits — JSON.stringify of a long transcript +
        // synchronous localStorage writes were the source of 100ms+ hitches on
        // slow machines. The final idle save is immediate so nothing is lost.
        if (session && hasContent(snapshot)) {
          const now = Date.now();
          if (!busyNow || now - lastPersist >= 2000) {
            lastPersist = now;
            const persistSession = session;
            const persistPrefixLen = historyPrefix ? historyPrefix.timeline.length : 0;
            const persist = () => {
              // Cache the latest snapshot in memory (never to disk); the cloud
              // push below is the durable copy.
              snapshotCache.set(persistSession.id, snapshot);
              const prev = get().conversations.find((c) => c.id === persistSession.id);
              const remoteHost = get().activeRemoteHost;
              // Project folder is the remote root for a remote session, else local.
              const project =
                persistSession.provider === "local"
                  ? (remoteHost ? get().activeRemote?.cwd : get().localSettings.cwd.trim()) ||
                    undefined
                  : undefined;
              // Only advance updatedAt when the timeline actually grew past the
              // restored prefix — i.e. real new activity. Merely opening/resuming a
              // conversation replays its transcript without adding turns, so its
              // order (and its whole project group's) must stay put in the sidebar.
              const grew = snapshot.timeline.length > persistPrefixLen;
              const meta: ConversationMeta = {
                id: persistSession.id,
                // A manual rename wins over the auto-derived title forever.
                title: prev?.titleLocked && prev.title ? prev.title : deriveTitle(snapshot),
                provider: persistSession.provider,
                mode: persistSession.mode,
                project: project ?? prev?.project,
                remoteHost: remoteHost ?? prev?.remoteHost,
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

        if (busyNow) dispatching = false; // a run is active again — clear the drain guard

        // Native notification on the busy→idle edge (desktop only, and only when
        // the window is unfocused — see notify()).
        if (prevBusy && !busyNow && session) {
          const failedRun = Object.values(snapshot.runs).some((r) => r.status === "failed");
          const title = get().conversations.find((c) => c.id === session.id)?.title;
          if (failedRun) {
            void notify("Run failed", title ? `“${title}” ended with an error.` : "The agent ended unexpectedly.");
          } else {
            void notify("Clark finished", title ? `“${title}” is ready for review.` : "Your task is ready for review.");
          }
          // The user was peeking at another conversation while this run streamed;
          // now that it settled (and its transcript just persisted above), promote
          // the peek to a real open so they can keep working there.
          const peeked = get().peek;
          if (peeked) {
            // Flush the idle snapshot synchronously — the raf-coalesced render
            // hasn't run yet, and openConversation's busy check must see the run
            // as settled or it would just re-peek.
            pending = null;
            set({ snapshot, peek: null });
            void get().openConversation(peeked.id);
          }
        }
        prevBusy = busyNow;

        // Refresh the credit balance shortly after a turn settles so the credit
        // banner reflects spend (throttled — billing is a network call).
        if (!busyNow && session) {
          const now = Date.now();
          if (now - lastBilling > 15000) {
            lastBilling = now;
            void get().loadBilling();
          }
        }

        // Auto-approve the pending permission per the current policy (Full access
        // grants everything; "Approve for me" grants all but destructive-looking
        // actions). Guarded so each request is answered exactly once.
        const pend = snapshot.pending_permission;
        if (pend) {
          if (pend.id !== autoResolvedId && wouldAutoApprove(get().permissionMode, pend)) {
            const opt = pickAllowOption(pend);
            const sess = get().session;
            if (opt && sess) {
              autoResolvedId = pend.id;
              bridge
                .respond(sess.id, { kind: "permission", request: pend.id, option: opt.id })
                .catch((e) => set({ error: String(e) }));
            }
          } else if (pend.id !== notifiedPermId && !wouldAutoApprove(get().permissionMode, pend)) {
            // The gate will actually block for the user — ping them.
            notifiedPermId = pend.id;
            void notify("Approval needed", pend.title || "Clark is waiting for your approval.");
          }
        } else {
          autoResolvedId = null;
          notifiedPermId = null;
        }

        // Drain the next queued message whenever idle and unblocked. Draining on
        // every idle snapshot (not just the busy→idle edge) means a permission
        // prompt open at the finish moment never strands the queue; `dispatching`
        // prevents a double-send before the new run shows up as busy.
        if (!busyNow && !snapshot.pending_permission && !dispatching) {
          const { queued, session: sess } = get();
          if (sess && queued.length > 0) {
            const [next, ...rest] = queued;
            dispatching = true;
            set({ queued: rest });
            bridge
              .prompt(sess.id, [{ type: "text", text: next.text }], next.uploads)
              .catch((e) => {
                set({ error: String(e) });
                dispatching = false;
              });
          }
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
      // Seed the cache so migrated chats open instantly, then upload snapshot +
      // archived state (idempotent; the server's rev guard won't clobber newer).
      snapshotCache.set(d.meta.id, d.snapshot);
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
      set({ loadingMemory: false, memoryOverview, globalMemoryOverview });
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
    get().endSession();
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
    const { bridge, activeProvider, auth, activeRemote: prevRemote } = get();
    if (!bridge || !activeProvider) return;
    // Replacing any prior remote connection; tear it down (best-effort).
    if (prevRemote) void sshDisconnect(prevRemote.id);
    set({ connecting: true, error: null, activeRemote: null, activeRemoteHost: null });
    let remote: RemoteInfo | null = null;
    try {
      const isLocal = activeProvider === "local";
      // Make sure a Clark Code key has been minted before the local provider
      // needs it (covers the case where sign-in's background provision is still
      // in flight or failed).
      if (isLocal) await get().ensureCodeKey();
      const localSettings = get().localSettings;
      const isRemote = isLocal && get().projectMode === "remote";

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

      await bridge.connect(activeProvider, config);
      const session = await bridge.newSession(activeProvider, options);
      if (isLocal && !isRemote && localSettings.cwd.trim()) {
        set({ recentProjects: addRecentProject(localSettings.cwd.trim()) });
      }
      set({
        session,
        connecting: false,
        historyPrefix: null,
        peek: null,
        queued: [],
        activeRemote: remote,
        activeRemoteHost: remoteHost,
      });
    } catch (e) {
      // Brought up a tunnel but failed afterward → tear it back down.
      if (remote) void sshDisconnect(remote.id);
      set({ error: String(e), connecting: false });
    }
  },

  endSession: () => {
    for (const a of get().attachments) if (a.previewUrl) URL.revokeObjectURL(a.previewUrl);
    const r = get().activeRemote;
    if (r) void sshDisconnect(r.id);
    set({
      session: null,
      snapshot: emptySnapshot(),
      error: null,
      attachments: [],
      historyPrefix: null,
      peek: null,
      opening: null,
      composerPrefill: null,
      queued: [],
      terminalOpen: false,
      activeRemote: null,
      activeRemoteHost: null,
    });
  },

  openConversation: async (id) => {
    const { bridge, activeProvider, auth, session, providers, localSettings, activeRemote: prevRemote } = get();
    if (!bridge || !activeProvider) return;
    if (session?.id === id) {
      // Returning to the live conversation just drops the peek — the stream was
      // running underneath the whole time.
      set({ peek: null });
      return;
    }
    // A run is streaming in the live conversation: don't tear it down — show the
    // other conversation read-only (peek). It promotes to a full open when the
    // run settles (see the busy→idle edge in init's subscribe handler).
    if (session && isBusy(get().snapshot)) {
      // Read-only peek while the live run streams: fetch the transcript from the
      // cloud (or the in-memory cache).
      const restored = await fetchSnapshot(id, get().auth);
      set({ peek: { id, snapshot: restored ?? emptySnapshot() } });
      return;
    }
    if (prevRemote) void sshDisconnect(prevRemote.id);
    const openingMeta = get().conversations.find((c) => c.id === id);
    set({
      connecting: true,
      error: null,
      dismissedFailedRuns: [],
      peek: null,
      opening: {
        id,
        title: openingMeta?.title || "Conversation",
        remoteHost: openingMeta?.remoteHost ?? null,
      },
      activeRemote: null,
      activeRemoteHost: null,
    });
    // Cloud-first: the transcript comes from the in-memory cache or a `cloudGet`.
    const restored = await fetchSnapshot(id, get().auth);
    let remote: RemoteInfo | null = null;
    try {
      const isLocal = activeProvider === "local";
      const canResume =
        providers.find((p) => p.id === activeProvider)?.capabilities.load_session ?? false;

      // A remote conversation reconnects its host (matched by SSH destination);
      // the saved host must still exist on this device.
      const meta = get().conversations.find((c) => c.id === id);
      const wantRemote = isLocal && !!meta?.remoteHost;
      let config;
      let options;
      let remoteHost: string | null = null;
      if (wantRemote) {
        const host = loadSshHosts().find((h) => h.host.trim() === meta!.remoteHost);
        if (!host) {
          throw new Error(`Add the SSH host "${meta!.remoteHost}" to reopen this remote conversation.`);
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

      await bridge.connect(activeProvider, config);
      // Providers that can't resume (the local agent has no server-side session)
      // reopen as a fresh session bound to the project; the saved transcript shows
      // as read-only history and new turns continue from there. Crucially, keep
      // the conversation's original id so it doesn't fork into a duplicate — the
      // local provider ignores the passed session id and uses its own internal
      // one, so the displayed id can stay stable.
      const opened = canResume
        ? await bridge.loadSession(activeProvider, id)
        : { ...(await bridge.newSession(activeProvider, options)), id };
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
      if (remote) void sshDisconnect(remote.id);
      set({ error: String(e), connecting: false, opening: null });
    }
  },

  archiveConversation: (id) => {
    // Soft-delete: flag it archived in the cloud (the transcript stays, so it
    // can be restored in full). Optimistic in-memory flag; PATCH to the cloud.
    const cleared = get().session?.id === id;
    if (cleared) {
      const r = get().activeRemote;
      if (r) void sshDisconnect(r.id);
    }
    set({
      conversations: get().conversations.map((c) => (c.id === id ? { ...c, archived: true } : c)),
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
    snapshotCache.delete(id);
    const cleared = get().session?.id === id;
    if (cleared) {
      const r = get().activeRemote;
      if (r) void sshDisconnect(r.id);
    }
    set({
      conversations: get().conversations.filter((c) => c.id !== id),
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
      await bridge.reconfigure(config);
    } catch (e) {
      set({ error: `Model switch failed: ${String(e)}` });
    }
  },

  setComposerPrefill: (text) => set({ composerPrefill: text }),

  shareConversation: async () => {
    const { session, peek, auth } = get();
    const id = peek?.id ?? session?.id;
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
    const { session, peek, auth } = get();
    const id = peek?.id ?? session?.id;
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
    const { bridge, session, attachments, snapshot, peek } = get();
    if (!bridge || !session) return;
    // Peeking at another conversation while a run streams: messages belong to the
    // live conversation only — the composer is disabled, this is a backstop.
    if (peek) return;
    if (!text.trim() && attachments.length === 0) return;
    const uploads = attachments.map(toUpload);
    for (const a of attachments) if (a.previewUrl) URL.revokeObjectURL(a.previewUrl);
    set({ attachments: [], error: null });
    // A run is active: queue this message instead of interrupting. It sends
    // automatically once the run finishes (drained in the subscribe handler).
    if (isBusy(snapshot)) {
      set((s) => ({
        queued: [...s.queued, { id: crypto.randomUUID(), text, uploads }],
      }));
      return;
    }
    try {
      await bridge.prompt(session.id, [{ type: "text", text }], uploads);
    } catch (e) {
      // Surface the failure instead of silently doing nothing.
      set({ error: String(e) });
    }
  },

  removeQueued: (id) => set((s) => ({ queued: s.queued.filter((q) => q.id !== id) })),

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
    await bridge.respond(session.id, response);
  },
}));

// Dev-only test seam: lets headless harnesses inject store state (e.g. a low
// credit balance) to exercise UI that depends on the live backend. Stripped from
// production builds.
if (import.meta.env.DEV && typeof window !== "undefined") {
  (window as unknown as { __clarkStore?: typeof useSessionStore }).__clarkStore =
    useSessionStore;
}
