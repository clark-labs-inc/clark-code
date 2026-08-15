import { deferredSnapshotPersistIsCurrent } from "../lib/snapshotPersistence";
import { approvalPolicyForSpecialist } from "../lib/permissions";
import { sidebarFixtureConversations, sidebarFixtureEnabled } from "../lib/sidebarFixture";
import {
  type SessionState,
  type ConversationMeta,
  type SessionGet,
  type SessionSet,
  type Snapshot,
  UPDATE_DRAIN_POLL_MS,
  UPDATE_DRAIN_TIMEOUT_MS,
  addRecentProject,
  authSignOut,
  beginUpdateDrain,
  cancelUpdateDrain,
  checkAndStageUpdate,
  closeLiveSession,
  cloudCreds,
  cloudList,
  codeKeyAccountBinding,
  configureCloudHistoryCredentials,
  consumeJustUpdated,
  delay,
  deriveTitle,
  effectiveApprovalPolicy,
  emptySnapshot,
  flushCloudPuts,
  getBridge,
  hasContent,
  appInitializationState,
  installStagedUpdate,
  isBusy,
  latestRunFailed,
  loadApprovalPolicy,
  loadApprovalPolicies,
  loadBrowserEnabled,
  loadChatModels,
  loadCollaborationMode,
  loadLocalSettings,
  loadMemoriesEnabled,
  loadOrchestrationEnabled,
  loadOutputStyle,
  loadRecentProjects,
  loadSshHosts,
  liveSessions,
  liveUpdateBlockerCount,
  markUnseenFinished,
  markAuthReconnectRequired,
  mergeConversations,
  mergeHistory,
  minLoadDuration,
  notify,
  nextSessionEpoch,
  onCloudHistoryConflict,
  onCloudHistoryWarning,
  onSettingsMenuRequested,
  onUpdateMenuRequested,
  pickAllowOption,
  pickFolder,
  provisionCodeKey,
  refreshAuthSession,
  refreshStagedUpdate,
  resetCloudHistory,
  relaunchApp,
  saveBrowserEnabled,
  saveLocalSettings,
  saveMemoriesEnabled,
  saveOrchestrationEnabled,
  scheduleCloudPut,
  settleRuns,
  signInWithGoogle,
  snapshotCache,
  syncFanOut,
  wouldAutoApprove,
} from "./sessionStore.runtime";
import { SPECIALIST_CATALOG_SHA256 } from "../lib/specialists";
import { composerDraftOwner, removeComposerDraft } from "../lib/composerDraft";
import { useSpecialistStore } from "./specialistStore";
import { resetStableOrder } from "../lib/stableOrder";
import { authAccountMatches } from "../lib/account";
import { loadManagedWorktreeBase } from "../lib/managedWorktreeSettings";

type AppActions = Pick<
  SessionState,
  | "checkForUpdate"
  | "applyUpdate"
  | "dismissJustUpdated"
  | "dismissError"
  | "flashNotice"
  | "dismissNotice"
  | "dismissWarning"
  | "dismissFailedRun"
  | "init"
  | "ensureCodeKey"
  | "syncCloudIndex"
  | "selectProvider"
  | "setLocalSettings"
  | "setProjectMode"
  | "setSelectedHostId"
  | "setProjectFolder"
  | "pickProjectFolder"
  | "setMemoriesEnabled"
  | "setBrowserEnabled"
  | "setOrchestrationEnabled"
  | "loadMemory"
  | "toggleMemoryViewer"
  | "setMemoryViewerOpen"
  | "signIn"
  | "reconnectAuth"
  | "signOutAuth"
>;

/** A refreshed token for the same stable account still owns an in-flight list
 * request. When older sessions have no stable identity, retain the stricter
 * token comparison so one account can never publish rows into another. */
export function cloudRequestStillOwned(
  startedWith: SessionState["auth"],
  current: SessionState["auth"],
): boolean {
  return authAccountMatches(startedWith, current);
}

/** Whether applying `next` would replace the account that owns local live
 * sessions and caches. The normal UI signs out before another account can sign
 * in, but keeping this boundary here makes the state safe for an auth handoff
 * or a programmatic re-auth too. A refreshed token for the same stable account
 * is deliberately not a switch. */
export function authAccountChanged(
  current: SessionState["auth"],
  next: NonNullable<SessionState["auth"]>,
): boolean {
  if (!current) return false;
  const currentOwner = codeKeyAccountBinding(current);
  const nextOwner = codeKeyAccountBinding(next);
  if (currentOwner && nextOwner) return currentOwner !== nextOwner;
  return true;
}

function activateSignedInAccount(
  set: SessionSet,
  get: SessionGet,
  auth: NonNullable<SessionState["auth"]>,
): void {
  const replacingAccount = authAccountChanged(get().auth, auth);
  const accountScope = codeKeyAccountBinding(auth);
  resetCloudHistory();
  resetStableOrder();
  if (replacingAccount) {
    get().endSession({ force: true });
    snapshotCache.clear();
  }
  useSpecialistStore.getState().setAccountScope(accountScope);
  configureCloudHistoryCredentials(cloudCreds(auth));
  const accountLocalSettings = loadLocalSettings(accountScope);
  set({
    auth,
    localSettings: accountLocalSettings,
    managedWorktreeBase: loadManagedWorktreeBase(accountScope, accountLocalSettings.cwd),
    chatModels: loadChatModels(accountScope),
    approvalPolicies: loadApprovalPolicies(accountScope),
    memoriesEnabled: loadMemoriesEnabled(accountScope),
    browserEnabled: loadBrowserEnabled(accountScope),
    orchestrationEnabled: loadOrchestrationEnabled(accountScope),
    approvalPolicy: loadApprovalPolicy(accountScope),
    collaborationMode: loadCollaborationMode(accountScope),
    outputStyle: loadOutputStyle(accountScope),
    selectedHostId: loadSshHosts(accountScope)[0]?.id ?? null,
    projectMode: "local",
    recentProjects: loadRecentProjects(accountScope),
    memoryStatus: null,
    memoryViewerOpen: false,
    loadingMemory: false,
    memoryOverview: null,
    globalMemoryOverview: null,
    pendingManagedWorktreePath: null,
    deferredSessionStartDraft: null,
    terminalLaunch: null,
    mcpOpen: false,
    sshOpen: false,
    newProjectOpen: false,
    paletteOpen: false,
    error: null,
    notice: null,
    warning: null,
    dismissedFailedRuns: [],
    conversations: [],
    conversationsLoading: true,
    runningIds: [],
    unseenWorkIds: [],
    selectedConversationIds: new Set(),
    mutatingConversationIds: new Set(),
    conversationMutation: null,
  });
  void get().ensureCodeKey();
  void get().syncCloudIndex();
}

export function handleCloudConversationDeleted(
  set: SessionSet,
  get: SessionGet,
  bridge: NonNullable<SessionState["bridge"]>,
  conversationId: string,
): void {
  const before = get();
  const wasLive = liveSessions.has(conversationId);
  const openingTarget = before.opening?.id === conversationId;
  const restoringTarget =
    before.conversationMutation?.kind === "restore"
    && before.mutatingConversationIds.has(conversationId);
  const viewTarget =
    before.session?.id === conversationId
    || before.unavailableConversation?.id === conversationId
    || openingTarget;
  const cleanupOwnsTarget =
    before.unavailableConversation?.id === conversationId
    && before.unavailableCleanupId === conversationId;

  closeLiveSession(bridge, conversationId);
  snapshotCache.delete(conversationId);
  removeComposerDraft(composerDraftOwner(before.auth?.user ?? null), conversationId);
  if (openingTarget || restoringTarget) nextSessionEpoch();
  if (viewTarget && !cleanupOwnsTarget) useSpecialistStore.getState().close();

  set((current) => {
    const cleanupStillOwnsTarget =
      current.unavailableConversation?.id === conversationId
      && current.unavailableCleanupId === conversationId;
    return {
      conversations: current.conversations.filter(
        (conversation) => conversation.id !== conversationId,
      ),
      runningIds: current.runningIds.filter((id) => id !== conversationId),
      warning:
        cleanupStillOwnsTarget
          ? current.warning
          : viewTarget || restoringTarget || wasLive
          ? "This conversation was deleted on another device, so Clark Code stopped it here."
          : current.warning,
      ...(viewTarget
        ? {
            session: null,
            unavailableConversation: cleanupStillOwnsTarget
              ? current.unavailableConversation
              : null,
            unavailableCleanupId: cleanupStillOwnsTarget
              ? current.unavailableCleanupId
              : null,
            opening: null,
            connecting: false,
            snapshot: settleRuns(emptySnapshot()),
            attachments: [],
            historyPrefix: null,
            composerPrefill: null,
            queued: [],
            terminalOpen: false,
            sideQuestion: null,
            activeRemote: null,
            activeRemoteHost: null,
            activeProjectRoot: null,
          }
        : {}),
    };
  });
}

export function handleCloudHistoryConflict(
  set: SessionSet,
  get: SessionGet,
  bridge: NonNullable<SessionState["bridge"]>,
  conversationId: string,
): void {
  snapshotCache.delete(conversationId);
  const before = get();
  const wasLive = liveSessions.has(conversationId);
  const cleanupOwnsTarget =
    before.unavailableConversation?.id === conversationId
    && before.unavailableCleanupId === conversationId;
  if (wasLive) closeLiveSession(bridge, conversationId);

  // An explicit deletion owns this target. A stale-write notification arriving
  // during that deletion must not replace cleanup with an undeletable refresh
  // screen while the durable delete is still in flight.
  if (cleanupOwnsTarget) {
    set({ runningIds: before.runningIds.filter((id) => id !== conversationId) });
    return;
  }

  const targeted =
    before.session?.id === conversationId
    || before.opening?.id === conversationId
    || before.unavailableConversation?.id === conversationId;
  if (targeted) {
    nextSessionEpoch();
    const meta = before.conversations.find(
      (conversation) => conversation.id === conversationId,
    );
    const autoReloadSpec = meta?.specialist?.kind === "spec";
    set({
      session: null,
      snapshot: emptySnapshot(),
      connecting: false,
      opening: null,
      unavailableConversation: {
        id: conversationId,
        title: meta?.title || "Conversation",
        detail: "Product cloud rejected a stale snapshot revision.",
        kind: "refresh_required",
      },
      unavailableCleanupId: null,
      attachments: [],
      historyPrefix: null,
      composerPrefill: null,
      queued: [],
      runningIds: before.runningIds.filter((id) => id !== conversationId),
      terminalOpen: false,
      sideQuestion: null,
      activeRemote: null,
      activeRemoteHost: null,
      activeProjectRoot: null,
      warning: null,
    });
    if (autoReloadSpec) {
      queueMicrotask(() => {
        const current = get();
        if (
          current.unavailableConversation?.id === conversationId
          && current.unavailableConversation.kind === "refresh_required"
        ) {
          void current.openConversation(conversationId);
        }
      });
    }
    return;
  }

  set({
    runningIds: before.runningIds.filter((id) => id !== conversationId),
    warning: wasLive
      ? "A conversation received a newer cloud revision, so Clark Code stopped its stale local session."
      : before.warning,
  });
}

export function createAppActions(set: SessionSet, get: SessionGet): AppActions {
  return {
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
      let waitedMs = 0;
      const hasVisibleWork = () => [...liveSessions.values()].some(
        (entry) => isBusy(entry.live) || !!entry.live.pending_permission || entry.queued.length > 0,
      );
      const waitForDrainPoll = async () => {
        // A real active run is allowed to finish without an arbitrary update
        // deadline. Bound only states that the UI cannot account for: a stale
        // prompt-start/queue flag or a native guard left after the UI settled.
        if (hasVisibleWork()) {
          waitedMs = 0;
          await delay(UPDATE_DRAIN_POLL_MS);
          return;
        }
        if (waitedMs >= UPDATE_DRAIN_TIMEOUT_MS) {
          throw new Error(
            "The update drain appears stuck. The update was cancelled so Clark Code can keep working; try again after reopening the app.",
          );
        }
        const pollMs = Math.min(UPDATE_DRAIN_POLL_MS, UPDATE_DRAIN_TIMEOUT_MS - waitedMs);
        await delay(pollMs);
        waitedMs += pollMs;
      };

      // Let current runs and already-queued follow-ups finish first. New sends
      // are rejected while `updateWaiting` is true, but permission responses and
      // cancellation remain available so a blocked run can still settle.
      while (get().connecting || liveUpdateBlockerCount() > 0) {
        await waitForDrainPoll();
      }

      // Close the final native race: latch prompt starts, then wait for any run
      // that entered just before the latch to release its RAII guard.
      while (true) {
        if (waitedMs >= UPDATE_DRAIN_TIMEOUT_MS) {
          throw new Error(
            "The native update drain appears stuck. The update was cancelled so Clark Code can keep working; try again after reopening the app.",
          );
        }
        if ((await beginUpdateDrain()) === 0) break;
        await waitForDrainPoll();
      }

      // A release may have landed while the update prompt was waiting. Keep
      // the native no-new-runs latch held, revalidate the latest pointer, and
      // replace a superseded staged payload before saving and installing.
      const refreshed = await refreshStagedUpdate((progress) => set({ updateProgress: progress }));
      set({ updateProgress: null });
      if (refreshed.status !== "ready") {
        throw new Error(
          refreshed.status === "error"
            ? refreshed.message
            : "Clark Code could not confirm the latest update; try again.",
        );
      }
      set({ update: refreshed.update });

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
        updateProgress: null,
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
  dismissFailedRun: (runId) =>
    set((s) =>
      s.dismissedFailedRuns.includes(runId)
        ? s
        : { dismissedFailedRuns: [...s.dismissedFailedRuns, runId] },
    ),

  init: () => {
    if (appInitializationState.initialization) return appInitializationState.initialization;
    appInitializationState.initialization = (async () => {
    // Native chrome and self-update are app-lifecycle concerns, not provider
    // concerns. Install them before provider discovery so a broken provider can
    // never suppress Settings or the recovery update path. Guards also prevent
    // React Strict Mode's development double-mount from duplicating timers.
    if (!appInitializationState.settingsMenuListenerInstalled) {
      appInitializationState.settingsMenuListenerInstalled = true;
      void onSettingsMenuRequested(() => get().setSettingsOpen(true)).catch(() => {
        appInitializationState.settingsMenuListenerInstalled = false;
      });
    }
    if (!appInitializationState.updateMenuListenerInstalled) {
      appInitializationState.updateMenuListenerInstalled = true;
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
        appInitializationState.updateMenuListenerInstalled = false;
      });
    }
    if (!appInitializationState.updateTimersInstalled) {
      appInitializationState.updateTimersInstalled = true;
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
      const specialistCatalog = await bridge.listSpecialistCatalog?.();
      if (
        specialistCatalog
        && (
          specialistCatalog.catalogSha256 !== SPECIALIST_CATALOG_SHA256
          || specialistCatalog.trust.source !== "signed_app_bundle"
          || specialistCatalog.trust.requiresSignedReleaseBinary !== true
        )
      ) {
        throw new Error(
          "Clark Code rejected a specialist catalog that did not match its signed native bundle.",
        );
      }
      configureCloudHistoryCredentials(cloudCreds(get().auth));
      // Native trajectory sync hit a 401 mid-retry: refresh the host-owned
      // Google/the agent credential generation. The retry loop reads it natively,
      // so the run self-heals without any WebView credential. Single-
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
            if (refreshed && authAccountMatches(get().auth, auth)) {
              configureCloudHistoryCredentials(cloudCreds(refreshed));
              set({ auth: refreshed });
              await get().syncCloudIndex();
            }
          } catch {
            const auth = get().auth;
            if (auth) {
              set({
                auth: markAuthReconnectRequired(auth),
                warning: "Your account needs reconnecting. Local work remains available.",
              });
            }
          } finally {
            refreshingCloudToken = false;
          }
        })();
      });
      // A network delivery warning means the SQLite prefix is safe and the run
      // can continue. A local journal failure is fail-closed by the native
      // bridge at the last durable event and uses this same warning surface.
      bridge.onCloudSyncWarning?.((message) => set({ warning: message }));
      bridge.onCloudConversationDeleted?.((conversationId) => {
        handleCloudConversationDeleted(set, get, bridge, conversationId);
      });
      onCloudHistoryConflict((conversationId) => {
        handleCloudHistoryConflict(set, get, bridge, conversationId);
      });
      onCloudHistoryWarning((message) => set({ warning: message }));
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
              // Sign-out, account replacement, archive, and delete remove this
              // exact entry before clearing their account-owned state. A
              // deferred frame from that entry must never repopulate the next
              // account's cache/sidebar or schedule against its credentials.
              if (liveSessions.get(id) !== entry) return;
              // Busy snapshots are deferred to a macrotask so streaming never
              // blocks the render frame. A terminal event can arrive before
              // that task runs; never let the older "running" projection land
              // after the newer idle snapshot and resurrect mobile activity.
              if (busyNow && !deferredSnapshotPersistIsCurrent(entry.live, live)) return;
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
                rev: prev?.rev,
                createdAt: prev?.createdAt ?? Date.now(),
                updatedAt: grew ? Date.now() : (prev?.updatedAt ?? Date.now()),
                archived: prev?.archived,
                specialist: prev?.specialist,
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
              // Mirror to the agent on the same throttle as local persistence:
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
          const failedRun = latestRunFailed(live);
          const title = get().conversations.find((c) => c.id === id)?.title;
          if (failedRun) {
            void notify("Run failed", title ? `“${title}” ended with an error.` : "The agent ended unexpectedly.");
          } else {
            void notify("the agent finished", title ? `“${title}” is ready for review.` : "Your task is ready for review.");
          }
        }
        // A run that just finished in a conversation the user isn't looking at
        // earns a blue "finished, not yet visited" dot in the sidebar until it's
        // opened. Neither the chat on screen nor an archived row gets the marker
        // (the first is being viewed, the second is hidden from the list).
        if (justSettled && !isActive) {
          const archived =
            get().conversations.find((conversation) => conversation.id === id)?.archived ?? false;
          const unseen = get().unseenWorkIds;
          const marked = markUnseenFinished(unseen, id, get().session?.id ?? null, archived);
          if (marked !== unseen) set({ unseenWorkIds: marked });
        }
        entry.prevBusy = busyNow;

        // Auto-approve the pending permission per THIS conversation's policy
        // (its own override, else the account's global default). Full access
        // grants everything; "Approve for me" grants all but destructive-looking
        // actions. Guarded so each request is answered exactly once — works for
        // background sessions too, so a run never stalls just because its
        // conversation isn't on screen.
        const pend = live.pending_permission;
        if (pend) {
          const specialistKind = get().conversations.find(
            (conversation) => conversation.id === id,
          )?.specialist?.kind;
          const policy = approvalPolicyForSpecialist(
            effectiveApprovalPolicy(get().approvalPolicy, get().approvalPolicies, id),
            specialistKind,
          );
          if (pend.id !== entry.autoResolvedId && wouldAutoApprove(policy, pend)) {
            const opt = pickAllowOption(pend);
            if (opt) {
              entry.autoResolvedId = pend.id;
              bridge
                .respond(id, { kind: "permission", request: pend.id, option: opt.id })
                .catch((e) => set({ error: String(e) }));
            }
          } else if (pend.id !== entry.notifiedPermId && !wouldAutoApprove(policy, pend)) {
            // The gate will actually block for the user — ping them.
            entry.notifiedPermId = pend.id;
            void notify("Approval needed", pend.title || "the agent is waiting for your approval.");
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
            .prompt(id, [{ type: "text", text: next.text }, ...next.skills], next.uploads)
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
      // Browser QA needs a stable long list, but it must never touch real cloud
      // history. This is development-only and requires an explicit query flag.
      if (sidebarFixtureEnabled()) {
        set({
          conversations: sidebarFixtureConversations(),
          conversationsLoading: false,
        });
        return;
      }
      // Best-effort: ensure the configured model key exists and pull
      // cloud/native-cached history. Both are no-ops offline or signed out.
      void get().ensureCodeKey();
      void get().syncCloudIndex();
    } catch (e) {
      set({ error: String(e) });
      appInitializationState.initialization = null;
    }
    })();
    return appInitializationState.initialization;
  },

  ensureCodeKey: async () => {
    const auth = get().auth;
    const creds = cloudCreds(auth);
    const owner = codeKeyAccountBinding(auth);
    if (!creds || !owner) return;
    try {
      await provisionCodeKey(creds);
    } catch {
      /* offline / backend not deployed — onboarding still works, the key is
         re-attempted on the next sign-in / session start */
    }
  },

  syncCloudIndex: async () => {
    const requestAuth = get().auth;
    const creds = cloudCreds(requestAuth);
    if (!creds) {
      set({ conversationsLoading: false });
      return;
    }
    try {
      const remote = await cloudList(creds);
      if (!cloudRequestStillOwned(requestAuth, get().auth)) return;
      // Cloud is authoritative; keep only in-memory-only entries (a just-migrated
      // chat whose push hasn't landed yet) so they don't flash out of the list.
      set({
        conversations: mergeConversations(remote, get().conversations),
        conversationsLoading: false,
      });
    } catch {
      // Offline / backend down — leave whatever's in memory, stop the spinner.
      if (cloudRequestStillOwned(requestAuth, get().auth)) {
        set({ conversationsLoading: false });
      }
    }
  },

  selectProvider: (id) => set({ activeProvider: id }),

  setLocalSettings: (patch) => {
    const current = get().localSettings;
    const next = { ...current, ...patch };
    saveLocalSettings(next, codeKeyAccountBinding(get().auth));
    const changedProject = patch.cwd !== undefined && patch.cwd.trim() !== current.cwd.trim();
    // A folder change invalidates any in-flight start/open that captured the
    // previous checkout. Keep the live-session pool intact, but make the
    // pending transition visibly cancelable instead of letting a late
    // worktree creation attach to the newly selected project.
    if (changedProject) nextSessionEpoch();
    set({
      localSettings: next,
      ...(changedProject
        ? {
            pendingManagedWorktreePath: null,
            deferredSessionStartDraft: null,
            worktreeTransition: null,
            dirtyWorktreeApproval: null,
            worktreePreparing: false,
            connecting: false,
            opening: null,
            managedWorktreeBase: loadManagedWorktreeBase(
              codeKeyAccountBinding(get().auth),
              next.cwd,
              useSpecialistStore.getState().active,
            ),
          }
        : {}),
    });
  },

  setProjectMode: (mode) => {
    const changedMode = mode !== get().projectMode;
    if (changedMode) nextSessionEpoch();
    set({
      projectMode: mode,
      error: null,
      ...(changedMode
        ? {
            pendingManagedWorktreePath: null,
            deferredSessionStartDraft: null,
            worktreeTransition: null,
            dirtyWorktreeApproval: null,
            worktreePreparing: false,
          }
        : {}),
      ...(changedMode ? { connecting: false, opening: null } : {}),
    });
  },
  setSelectedHostId: (id) => {
    const changedHost = id !== get().selectedHostId;
    if (changedHost) nextSessionEpoch();
    set({
      selectedHostId: id,
      ...(changedHost
        ? {
            dirtyWorktreeApproval: null,
            deferredSessionStartDraft: null,
            connecting: false,
            opening: null,
          }
        : {}),
    });
  },

  setProjectFolder: (path) => {
    get().setLocalSettings({ cwd: path });
    set({
      recentProjects: addRecentProject(path, codeKeyAccountBinding(get().auth)),
      memoryStatus: null,
      memoryOverview: null,
    });
  },

  pickProjectFolder: async () => {
    const requestAuth = get().auth;
    try {
      const picked = await pickFolder(get().localSettings.cwd || undefined);
      if (picked && authAccountMatches(requestAuth, get().auth)) get().setProjectFolder(picked);
    } catch (e) {
      if (!authAccountMatches(requestAuth, get().auth)) return;
      set({ error: String(e) });
    }
  },

  setMemoriesEnabled: (on) => {
    saveMemoriesEnabled(on, codeKeyAccountBinding(get().auth));
    set({ memoriesEnabled: on });
  },

  setBrowserEnabled: (on) => {
    saveBrowserEnabled(on, codeKeyAccountBinding(get().auth));
    set({ browserEnabled: on });
  },

  setOrchestrationEnabled: (on) => {
    saveOrchestrationEnabled(on, codeKeyAccountBinding(get().auth));
    set({ orchestrationEnabled: on });
  },

  loadMemory: async () => {
    const { bridge, session } = get();
    const accountScope = codeKeyAccountBinding(get().auth);
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
          session ? bridge.listMemory(session.id) : Promise.resolve(null),
          accountScope && bridge.listGlobalMemory
            ? bridge.listGlobalMemory()
            : Promise.resolve(null),
        ]),
      );
      if (codeKeyAccountBinding(get().auth) !== accountScope) return;
      set({ loadingMemory: false, memoryOverview, globalMemoryOverview, memoryStatus: null });
    } catch (e) {
      if (codeKeyAccountBinding(get().auth) !== accountScope) return;
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
    activateSignedInAccount(set, get, auth);
  },

  reconnectAuth: async () => {
    const current = get().auth;
    if (!current) return;
    try {
      const refreshed = await refreshAuthSession(current);
      if (!authAccountMatches(get().auth, current)) return;
      configureCloudHistoryCredentials(cloudCreds(refreshed));
      set({ auth: refreshed, error: null, warning: null });
      await get().syncCloudIndex();
      return;
    } catch {
      // A retained refresh token may no longer be usable. Re-authenticate the
      // same account interactively without destroying its local task state.
    }
    const reauthenticated = await signInWithGoogle();
    if (!authAccountMatches(reauthenticated, current)) {
      activateSignedInAccount(set, get, reauthenticated);
      return;
    }
    configureCloudHistoryCredentials(cloudCreds(reauthenticated));
    set({ auth: reauthenticated, error: null, warning: null });
    await get().syncCloudIndex();
  },

  signOutAuth: async () => {
    // A Clark Code key is account-scoped. Removing the local binding prevents
    // the next user from silently consuming access owned by this account.
    resetCloudHistory();
    resetStableOrder();
    try {
      await authSignOut();
    } catch (error) {
      set({ error: `Could not sign out safely: ${String(error)}` });
      return;
    }
    get().endSession({ force: true });
    useSpecialistStore.getState().setAccountScope(null);
    // Drop the in-memory history entirely so the signed-out (and any next)
    // account starts clean.
    snapshotCache.clear();
    const anonymousLocalSettings = loadLocalSettings(null);
    set({
      auth: null,
      localSettings: anonymousLocalSettings,
      managedWorktreeBase: loadManagedWorktreeBase(null, anonymousLocalSettings.cwd),
      chatModels: loadChatModels(null),
      approvalPolicies: loadApprovalPolicies(null),
      memoriesEnabled: loadMemoriesEnabled(null),
      browserEnabled: loadBrowserEnabled(null),
      orchestrationEnabled: loadOrchestrationEnabled(null),
      approvalPolicy: loadApprovalPolicy(null),
      collaborationMode: loadCollaborationMode(null),
      outputStyle: loadOutputStyle(null),
      selectedHostId: loadSshHosts(null)[0]?.id ?? null,
      projectMode: "local",
      recentProjects: [],
      memoryStatus: null,
      memoryViewerOpen: false,
      loadingMemory: false,
      memoryOverview: null,
      globalMemoryOverview: null,
      pendingManagedWorktreePath: null,
      deferredSessionStartDraft: null,
      terminalLaunch: null,
      mcpOpen: false,
      sshOpen: false,
      sshOpenPurpose: "manage",
      newProjectOpen: false,
      paletteOpen: false,
      error: null,
      notice: null,
      warning: null,
      dismissedFailedRuns: [],
      conversations: [],
      conversationsLoading: false,
      unseenWorkIds: [],
      selectedConversationIds: new Set(),
      mutatingConversationIds: new Set(),
      conversationMutation: null,
    });
  },

  };
}
