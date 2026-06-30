import { create } from "zustand";
import { getBridge, type CoreBridge } from "../core-bridge/bridge";
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
  loadIndex,
  loadSnapshot,
  saveSnapshot,
  upsertMeta,
  deleteConversation,
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
  type PermissionMode,
} from "../lib/permissions";
import {
  cloudCreds,
  cloudList,
  cloudGet,
  scheduleCloudPut,
  cloudDelete,
} from "../lib/cloudHistory";
import { provisionCodeKey, billingMe, type BillingSummary } from "../lib/account";
import { notify } from "../lib/notify";
import { checkAndStageUpdate, relaunchApp, type StagedUpdate } from "../lib/updater";

/** A follow-up message the user sent while a run was active. It sends
 *  automatically when the run finishes — Codex-style, never interrupting. */
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
  /** Authenticated user + the Clark connection config it carries. */
  auth: AuthSession | null;
  /** Files staged to send with the next message. */
  attachments: PendingAttachment[];
  /** Saved conversations, newest first. */
  conversations: ConversationMeta[];
  /** Restored transcript when a past conversation is reopened (prefix to live). */
  historyPrefix: Snapshot | null;
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
  /** True while Clark is extracting the per-repo project memory. */
  extractingMemory: boolean;
  /** Last memory-extraction status message (success or error). */
  memoryStatus: string | null;
  /** Whether the per-folder memory viewer popover is open. */
  memoryViewerOpen: boolean;
  /** True while the memory viewer is (re)loading the folder's memory. */
  loadingMemory: boolean;
  /** The last-loaded memory for the active project folder. */
  memoryOverview: MemoryOverview | null;
  /** Recently opened project folders (newest first). */
  recentProjects: string[];
  /** Follow-up messages sent while a run is active; drained when it finishes. */
  queued: QueuedMessage[];
  /** How agent permission requests are approved (Codex-style). */
  permissionMode: PermissionMode;
  /** Whether the in-chat terminal drawer is open. */
  terminalOpen: boolean;
  /** Whether the MCP servers settings modal is open. */
  mcpOpen: boolean;
  /** Whether the remote-hosts (SSH) settings modal is open. */
  sshOpen: boolean;
  /** Whether the ⌘K command palette is open. */
  paletteOpen: boolean;
  /** Whether the sidebar is collapsed to its icon rail. */
  sidebarCollapsed: boolean;
  /** Billing summary (plan, subscription, credits) from Clark; null until loaded. */
  billing: BillingSummary | null;
  loadingBilling: boolean;
  /** A downloaded + staged app update awaiting a relaunch to apply. */
  update: StagedUpdate | null;

  init: () => Promise<void>;
  loadBilling: () => Promise<void>;
  selectProvider: (id: string) => void;
  setLocalSettings: (patch: Partial<LocalAgentSettings>) => void;
  setProjectMode: (mode: "local" | "remote") => void;
  setSelectedHostId: (id: string | null) => void;
  setProjectFolder: (path: string) => void;
  pickProjectFolder: () => Promise<void>;
  extractMemory: () => Promise<void>;
  loadMemory: () => Promise<void>;
  toggleMemoryViewer: () => void;
  setMemoryViewerOpen: (open: boolean) => void;
  signIn: (method: AuthMethod) => Promise<void>;
  signOutAuth: () => void;
  /** Mint + store a Clark Code API key for the signed-in user if none yet. */
  ensureCodeKey: () => Promise<void>;
  /** Pull the cloud conversation list (Clark) and merge into the local index. */
  syncCloudIndex: () => Promise<void>;
  startSession: () => Promise<void>;
  endSession: () => void;
  openConversation: (id: string) => Promise<void>;
  removeConversation: (id: string) => void;
  addFiles: (files: File[]) => Promise<void>;
  removeAttachment: (id: string) => void;
  send: (text: string) => Promise<void>;
  removeQueued: (id: string) => void;
  setPermissionMode: (mode: PermissionMode) => void;
  toggleTerminal: () => void;
  setTerminalOpen: (open: boolean) => void;
  setMcpOpen: (open: boolean) => void;
  setSshOpen: (open: boolean) => void;
  setPaletteOpen: (open: boolean) => void;
  togglePalette: () => void;
  /** Check for, download, verify, and stage a newer version (no-op outside the app). */
  checkForUpdate: () => Promise<void>;
  /** Relaunch into the staged update. */
  applyUpdate: () => Promise<void>;
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

export const useSessionStore = create<SessionState>((set, get) => ({
  bridge: null,
  providers: [],
  activeProvider: null,
  session: null,
  snapshot: emptySnapshot(),
  connecting: false,
  error: null,
  auth: loadAuthSession(),
  attachments: [],
  conversations: loadIndex(),
  historyPrefix: null,
  localSettings: loadLocalSettings(),
  projectMode: "local",
  selectedHostId: loadSshHosts()[0]?.id ?? null,
  activeRemote: null,
  activeRemoteHost: null,
  extractingMemory: false,
  memoryStatus: null,
  memoryViewerOpen: false,
  loadingMemory: false,
  memoryOverview: null,
  recentProjects: loadRecentProjects(),
  queued: [],
  permissionMode: loadPermissionMode(),
  terminalOpen: false,
  mcpOpen: false,
  sshOpen: false,
  paletteOpen: false,
  sidebarCollapsed: false,
  billing: null,
  loadingBilling: false,
  update: null,

  checkForUpdate: async () => {
    if (get().update) return; // already staged
    const staged = await checkAndStageUpdate();
    if (staged) set({ update: staged });
  },
  applyUpdate: async () => {
    await relaunchApp();
  },

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
      let pending: Snapshot | null = null;
      let rafScheduled = false;
      const flushRender = () => {
        rafScheduled = false;
        if (pending) {
          set({ snapshot: pending });
          pending = null;
        }
      };
      let lastPersist = 0;
      let lastBilling = 0;
      // Fold each engine snapshot into the active conversation: merge with any
      // restored history prefix, show it, and persist it so the chat survives a
      // restart and can be reopened later.
      bridge.subscribe((live) => {
        const { historyPrefix, session } = get();
        const snapshot = historyPrefix ? mergeHistory(historyPrefix, live) : live;

        // Render: coalesce to the next animation frame.
        pending = snapshot;
        if (!rafScheduled) {
          rafScheduled = true;
          raf(flushRender);
        }

        const busyNow = isBusy(snapshot);

        // Persist + sidebar meta: throttled while streaming, immediate when idle.
        if (session && hasContent(snapshot)) {
          const now = Date.now();
          if (!busyNow || now - lastPersist >= 450) {
            lastPersist = now;
            saveSnapshot(session.id, snapshot);
            const prev = get().conversations.find((c) => c.id === session.id);
            const remoteHost = get().activeRemoteHost;
            // Project folder is the remote root for a remote session, else local.
            const project =
              session.provider === "local"
                ? (remoteHost ? get().activeRemote?.cwd : get().localSettings.cwd.trim()) ||
                  undefined
                : undefined;
            const meta: ConversationMeta = {
              id: session.id,
              title: deriveTitle(snapshot),
              provider: session.provider,
              mode: session.mode,
              project: project ?? prev?.project,
              remoteHost: remoteHost ?? prev?.remoteHost,
              createdAt: prev?.createdAt ?? Date.now(),
              updatedAt: Date.now(),
            };
            upsertMeta(meta);
            set({ conversations: loadIndex() });
            // Mirror the turn to Clark once it settles — not every streamed frame.
            // Coalesced + single-flight + idempotent (see cloudHistory).
            if (!busyNow) {
              const creds = cloudCreds(get().auth);
              if (creds) scheduleCloudPut(creds, meta, snapshot);
            }
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
      // Best-effort: ensure a Clark Code key exists, pull cloud history, and load
      // the credit balance for the banner. All no-op offline / signed out.
      void get().ensureCodeKey();
      void get().syncCloudIndex();
      void get().loadBilling();
      // Auto-update: check shortly after launch, then every 6h. Downloads +
      // verifies + stages in the background; the UI surfaces "Restart to update".
      setTimeout(() => void get().checkForUpdate(), 4000);
      setInterval(() => void get().checkForUpdate(), 6 * 60 * 60 * 1000);
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
    if (!creds) return;
    try {
      const remote = await cloudList(creds);
      for (const meta of remote) {
        const prev = get().conversations.find((c) => c.id === meta.id);
        // Cloud wins for metadata unless the local copy is strictly newer.
        if (!prev || meta.updatedAt >= prev.updatedAt) upsertMeta(meta);
      }
      set({ conversations: loadIndex() });
    } catch {
      /* offline or backend not deployed — local cache still serves history */
    }
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

  extractMemory: async () => {
    const { bridge, localSettings: s } = get();
    if (!bridge?.extractMemory) {
      set({ memoryStatus: "Memory extraction needs the desktop app." });
      return;
    }
    if (!s.cwd.trim() || !s.apiKey.trim()) {
      set({ memoryStatus: "Choose a project folder and add your Clark API key first." });
      return;
    }
    set({ extractingMemory: true, memoryStatus: "Clark is analyzing the repository…" });
    try {
      await bridge.extractMemory(s.cwd.trim(), s.apiKey.trim(), s.model.trim() || "clark");
      set({ extractingMemory: false, memoryStatus: "Saved project memory to .clark/memory/MEMORY.md" });
      await get().loadMemory();
    } catch (e) {
      set({ extractingMemory: false, memoryStatus: `Extraction failed: ${String(e)}` });
    }
  },

  loadMemory: async () => {
    const { bridge, localSettings: s } = get();
    const cwd = s.cwd.trim();
    if (!bridge?.listMemory) {
      set({ memoryOverview: null });
      return;
    }
    if (!cwd) {
      set({ memoryOverview: null, memoryStatus: "Choose a project folder first." });
      return;
    }
    set({ loadingMemory: true });
    try {
      const memoryOverview = await bridge.listMemory(cwd);
      set({ loadingMemory: false, memoryOverview });
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
    set({ auth });
    // Provision the Clark Code key + pull cloud history so the user never has to
    // paste a key.
    void get().ensureCodeKey();
    void get().syncCloudIndex();
  },

  signOutAuth: () => {
    authSignOut();
    get().endSession();
    set({ auth: null, billing: null });
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
      queued: [],
      terminalOpen: false,
      activeRemote: null,
      activeRemoteHost: null,
      conversations: loadIndex(),
    });
  },

  openConversation: async (id) => {
    const { bridge, activeProvider, auth, session, providers, localSettings, activeRemote: prevRemote } = get();
    if (!bridge || !activeProvider) return;
    if (session?.id === id) return; // already open
    if (prevRemote) void sshDisconnect(prevRemote.id);
    let restored = loadSnapshot(id);
    set({ connecting: true, error: null, activeRemote: null, activeRemoteHost: null });
    // Not cached locally (e.g. opened on another machine)? Pull it from Clark.
    if (!restored) {
      const creds = cloudCreds(get().auth);
      if (creds) {
        try {
          const cloud = await cloudGet(creds, id);
          if (cloud) {
            restored = cloud;
            saveSnapshot(id, cloud); // cache for next time
          }
        } catch {
          /* offline — fall back to a fresh session */
        }
      }
    }
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
        attachments: [],
        queued: [],
        activeRemote: remote,
        activeRemoteHost: remoteHost,
      });
    } catch (e) {
      if (remote) void sshDisconnect(remote.id);
      set({ error: String(e), connecting: false });
    }
  },

  removeConversation: (id) => {
    deleteConversation(id);
    const creds = cloudCreds(get().auth);
    if (creds) cloudDelete(creds, id).catch(() => {});
    const cleared = get().session?.id === id;
    set({
      conversations: loadIndex(),
      ...(cleared
        ? { session: null, snapshot: emptySnapshot(), historyPrefix: null, queued: [] }
        : {}),
    });
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

  toggleTerminal: () => set((s) => ({ terminalOpen: !s.terminalOpen })),
  setTerminalOpen: (open) => set({ terminalOpen: open }),
  setMcpOpen: (open) => set({ mcpOpen: open }),
  setSshOpen: (open) => set({ sshOpen: open }),
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
