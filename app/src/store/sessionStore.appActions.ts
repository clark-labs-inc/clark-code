import {
  type SessionState,
  type ConversationMeta,
  type SessionGet,
  type SessionSet,
  type Snapshot,
  UPDATE_DRAIN_POLL_MS,
  addRecentProject,
  authSignOut,
  beginUpdateDrain,
  billingMe,
  cancelUpdateDrain,
  checkAndStageUpdate,
  closeLiveSession,
  cloudCreds,
  cloudList,
  cloudSetArchived,
  codeKeyAccountBinding,
  codeKeyMatchesAccount,
  codeKeyProvisions,
  configureCloudHistoryCredentials,
  consumeJustUpdated,
  delay,
  deriveTitle,
  drainLocalHistory,
  emptySnapshot,
  flushCloudPuts,
  getBridge,
  hasContent,
  hasUnscopedLocalHistory,
  hasSeenActivityReward,
  appInitializationState,
  installStagedUpdate,
  isBusy,
  latestActivityReward,
  latestRunFailed,
  liveSessions,
  liveUpdateBlockerCount,
  markActivityRewardSeen,
  mergeConversations,
  mergeHistory,
  minLoadDuration,
  notify,
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

type AppActions = Pick<
  SessionState,
  | "checkForUpdate"
  | "applyUpdate"
  | "dismissJustUpdated"
  | "dismissError"
  | "flashNotice"
  | "dismissNotice"
  | "dismissWarning"
  | "dismissActivityReward"
  | "dismissFailedRun"
  | "loadBilling"
  | "init"
  | "ensureCodeKey"
  | "syncCloudIndex"
  | "migrateLocalToCloud"
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
  | "signOutAuth"
>;

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
      // Billing is authoritative account state. Publish it before deriving the
      // optional reward presentation so a malformed/older reward field can
      // never turn a valid plan and balance into the empty-account fallback.
      set({ billing, loadingBilling: false });
      const reward = latestActivityReward(billing);
      const current = get().activityReward;
      const activityReward =
        current ?? (reward && !hasSeenActivityReward(get().auth, reward) ? reward : null);
      set({ activityReward });
    } catch {
      set({ loadingBilling: false });
    }
  },

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
      configureCloudHistoryCredentials(cloudCreds(get().auth));
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
            const refreshedToken = refreshed?.clark.token;
            if (refreshed && refreshedToken && get().auth?.clark.token === auth?.clark.token) {
              await bridge.refreshCloudSession?.(refreshedToken);
              configureCloudHistoryCredentials(cloudCreds(refreshed));
              set({ auth: refreshed });
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
        closeLiveSession(bridge, conversationId);
        snapshotCache.delete(conversationId);
        const active = get().session?.id === conversationId;
        set((current) => ({
          conversations: current.conversations.filter((conversation) => conversation.id !== conversationId),
          runningIds: current.runningIds.filter((id) => id !== conversationId),
          warning: "This conversation was deleted on another device, so Clark Code stopped it here.",
          ...(active
            ? {
                session: null,
                snapshot: settleRuns(emptySnapshot()),
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
      });
      onCloudHistoryConflict((conversationId) => {
        snapshotCache.delete(conversationId);
        set({
          warning: "This conversation changed on another device. Reopen it to continue from Clark cloud’s latest history.",
        });
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
          const failedRun = latestRunFailed(live);
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
      // Best-effort: ensure a Clark Code key exists, migrate any residual local
      // chats into the cloud (one-time), pull cloud history, and load the credit
      // balance. All no-op offline / signed out.
      void get().ensureCodeKey();
      get().migrateLocalToCloud();
      void get().syncCloudIndex();
      void get().loadBilling();
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
    const settings = get().localSettings;
    if (codeKeyMatchesAccount(settings.apiKey, settings.apiKeyOwner, auth)) return;
    if (settings.apiKey || settings.apiKeyOwner) {
      // Never make even one request with a legacy or cross-account key while
      // its replacement is being minted.
      get().setLocalSettings({ apiKey: "", apiKeyOwner: "" });
    }

    let provision = codeKeyProvisions.get(owner);
    if (!provision) {
      provision = provisionCodeKey(creds);
      codeKeyProvisions.set(owner, provision);
      const clearProvision = () => {
        if (codeKeyProvisions.get(owner) === provision) codeKeyProvisions.delete(owner);
      };
      void provision.then(clearProvision, clearProvision);
    }
    try {
      const key = await provision;
      // Never attach a key minted for an account that signed out or switched
      // while provisioning was in flight.
      if (key && codeKeyAccountBinding(get().auth) === owner) {
        get().setLocalSettings({ apiKey: key, apiKeyOwner: owner });
      }
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
    const auth = get().auth;
    const creds = cloudCreds(auth);
    if (!creds || !auth) return; // no cloud target yet — keep local data for a later sign-in
    const scopes = [auth.user.id, auth.user.email]
      .filter((scope): scope is string => typeof scope === "string" && scope.trim().length > 0);
    const drained = drainLocalHistory(scopes);
    if (hasUnscopedLocalHistory()) {
      set({
        warning: "Older local Clark Code history could not be matched to this account, so it was not uploaded.",
      });
    }
    if (drained.length === 0) return;
    for (const d of drained) {
      // Seed the cache so migrated chats open instantly (settled — a drained
      // transcript may have been persisted mid-run), then upload snapshot +
      // archived state (idempotent; the server's rev guard won't clobber newer).
      const settled = settleRuns(d.snapshot);
      snapshotCache.set(d.meta.id, settled);
      scheduleCloudPut(creds, d.meta, settled);
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
    const { bridge, session } = get();
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
    // A new authenticated principal starts with no inherited retry queue or
    // cached write revision from the prior account.
    resetCloudHistory();
    configureCloudHistoryCredentials(cloudCreds(auth));
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
    // A Clark Code key is account-scoped. Removing the local binding prevents
    // the next user from silently billing against this account.
    const cloudToken = get().auth?.clark.token;
    get().setLocalSettings({ apiKey: "", apiKeyOwner: "" });
    resetCloudHistory();
    authSignOut();
    if (cloudToken) void get().bridge?.clearCloudSession?.(cloudToken);
    get().endSession({ force: true });
    // Drop the in-memory history entirely so the signed-out (and any next)
    // account starts clean.
    snapshotCache.clear();
    set({ auth: null, billing: null, activityReward: null, conversations: [], conversationsLoading: false });
  },

  };
}
