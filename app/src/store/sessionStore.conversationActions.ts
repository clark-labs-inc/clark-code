import {
  type SessionState,
  type ConversationMeta,
  type RemoteInfo,
  type Session,
  type SessionGet,
  type SessionOptions,
  type SessionSet,
  addRecentProject,
  bindCloudTrajectory,
  buildResumeTranscript,
  closeLiveSession,
  cloudCreds,
  conversationProjectRoot,
  effectiveModelSettings,
  emptySnapshot,
  epochStale,
  fetchSnapshot,
  hostReady,
  liveProjectRoot,
  liveSessions,
  loadSshHosts,
  localConnectConfig,
  localSettingsReady,
  mergedOf,
  newLiveEntry,
  nextSessionEpoch,
  openRemote,
  pinChatModel,
  remoteTarget,
  resetFanOut,
  scheduleCloudPut,
  sshDisconnect,
  syncFanOut,
} from "./sessionStore.runtime";
import { saveManagedWorktreeBase } from "../lib/managedWorktreeSettings";
import { composerDraftRef } from "../lib/composerDraft";
import { createSidebarConversationActions } from "./sessionStore.sidebarConversationActions";

type ConversationActions = Pick<
  SessionState,
  | "startBlockedReason"
  | "startSession"
  | "setManagedWorktreeBase"
  | "confirmManagedWorktreeStart"
  | "dismissManagedWorktreeStart"
  | "endSession"
  | "openConversation"
  | "archiveConversation"
  | "restoreConversation"
  | "deleteConversation"
  | "renameConversation"
  | "toggleConversationSelection"
  | "setConversationSelection"
  | "archiveSelectedConversations"
  | "deleteSelectedConversations"
>;

export function createConversationActions(set: SessionSet, get: SessionGet): ConversationActions {
  return {
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

  setManagedWorktreeBase: (base) => {
    saveManagedWorktreeBase(base);
    set({ managedWorktreeBase: base });
  },

  confirmManagedWorktreeStart: async () => {
    const { bridge, worktreeTransition, managedWorktreeBase } = get();
    if (!bridge?.createManagedWorktree || !worktreeTransition) return;
    if (
      worktreeTransition.action !== "create_isolated"
      && worktreeTransition.action !== "preserve_changes"
    ) {
      set({
        error: "This branch transition needs a different checkout choice before starting a session.",
      });
      return;
    }
    const targetBranch = worktreeTransition.action === "preserve_changes"
      ? worktreeTransition.targetBranch ?? null
      : null;
    if (worktreeTransition.action === "preserve_changes" && !targetBranch) {
      set({ error: "The requested branch is no longer available for an isolated continuation." });
      return;
    }
    set({ worktreePreparing: true, error: null });
    try {
      const created = await bridge.createManagedWorktree(worktreeTransition.sourceRoot, {
        base: managedWorktreeBase,
        targetBranch,
      });
      set({
        pendingManagedWorktreePath: created.path,
        worktreeTransition: null,
        worktreePreparing: false,
      });
      await get().startSession();
    } catch (error) {
      set({ error: String(error), worktreePreparing: false });
    }
  },

  dismissManagedWorktreeStart: () => {
    set({ worktreeTransition: null, worktreePreparing: false });
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
      let localSessionPath = localSettings.cwd.trim();
      if (isLocal && !isRemote) {
        const pendingManagedWorktreePath = get().pendingManagedWorktreePath;
        if (pendingManagedWorktreePath) {
          localSessionPath = pendingManagedWorktreePath;
        } else if (
          bridge.projectContext
          && bridge.planProjectWorktree
          && bridge.createManagedWorktree
        ) {
          const context = await bridge.projectContext(localSessionPath);
          if (epochStale(epoch)) return;
          if (context) {
            const plan = await bridge.planProjectWorktree(localSessionPath);
            if (epochStale(epoch)) return;
            if (plan.action !== "create_isolated") {
              throw new Error(
                "This checkout needs a safe branch transition before Clark can start an isolated session.",
              );
            }
            if (!plan.sourceIsManaged && plan.requiresConfirmation) {
              set({
                connecting: false,
                opening: null,
                worktreeTransition: plan,
                worktreePreparing: false,
              });
              return;
            }
            if (!plan.sourceIsManaged) {
              set({ worktreePreparing: true });
              const created = await bridge.createManagedWorktree(localSessionPath, {
                base: get().managedWorktreeBase,
              });
              if (epochStale(epoch)) return;
              localSessionPath = created.path;
              set({
                pendingManagedWorktreePath: created.path,
                worktreePreparing: false,
              });
            }
          }
        }
      }

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
        const sessionSettings = { ...localSettings, cwd: localSessionPath };
        config = localConnectConfig(sessionSettings);
        options = { cwd: localSessionPath, mode, collaboration_mode };
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
        ? (isRemote ? remote?.cwd : localSessionPath) || undefined
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
        emptySnapshot(),
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
        pendingManagedWorktreePath: null,
        worktreePreparing: false,
        worktreeTransition: null,
      });
    } catch (e) {
      // Brought up a tunnel but failed afterward → tear it back down.
      if (nativeSession) void bridge.closeSession?.(nativeSession.id);
      if (remote) void sshDisconnect(remote.id);
      if (epochStale(epoch)) return;
      set({ error: String(e), connecting: false, opening: null, worktreePreparing: false });
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
    // Starting a new session unmounts the active-session composer and mounts a
    // fresh start-screen one, which would drop a half-typed draft (the text
    // lives in the composer's local state). Stage the mirrored draft as a
    // prefill so the new composer restores it. Sign-out (force) discards it for
    // a clean slate — the next account starts with an empty composer.
    const preservedDraft = opts?.force ? "" : composerDraftRef.current;
    if (opts?.force) composerDraftRef.current = "";
    set({
      session: null,
      snapshot: emptySnapshot(),
      error: null,
      connecting: false,
      attachments: [],
      historyPrefix: null,
      opening: null,
      composerPrefill: preservedDraft.trim() ? { text: preservedDraft } : null,
      queued: [],
      terminalOpen: false,
      sideQuestion: null,
      activeRemote: null,
      activeRemoteHost: null,
      activeProjectRoot: null,
      worktreeTransition: null,
      worktreePreparing: false,
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
      }, restored ?? emptySnapshot());
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
        warning: restored?.sync_pending
          ? "This conversation was recovered from local disk and will sync when Clark cloud is reachable."
          : get().warning,
      });
    } catch (e) {
      if (nativeSession) void bridge.closeSession?.(nativeSession.id);
      if (remote) void sshDisconnect(remote.id);
      if (epochStale(epoch)) return;
      set({ error: String(e), connecting: false, opening: null });
    }
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

  ...createSidebarConversationActions(set, get),

  };
}
