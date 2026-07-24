import {
  type SessionState,
  type ClientResponse,
  type ConversationMeta,
  type Session,
  type SessionGet,
  type SessionOptions,
  type SkillReferenceBlock,
  type SessionSet,
  MAX_ATTACHMENT_BYTES,
  bindCloudTrajectory,
  buildResumeTranscript,
  cloudCreds,
  cloudShare,
  cloudUnshare,
  copyText,
  deriveTitle,
  effectiveModelSettings,
  emptySnapshot,
  fileToAttachment,
  isBusy,
  liveSessions,
  localConnectConfig,
  newLiveEntry,
  nextApprovalPolicy,
  normalizeCodingModel,
  normalizeReasoningEffort,
  notify,
  pickAllowOption,
  pickFolder,
  remoteTarget,
  resetFanOut,
  saveApprovalPolicy,
  saveChatModels,
  saveCollaborationMode,
  saveLocalSettings,
  saveOutputStyle,
  scheduleCloudPut,
  snapshotBeforeTimelineItem,
  snapshotCache,
  sshDisconnect,
  toUpload,
  wouldAutoApprove,
} from "./sessionStore.runtime";

type InteractionActions = Pick<
  SessionState,
  | "updateModelSettings"
  | "setComposerPrefill"
  | "shareConversation"
  | "unshareConversation"
  | "addFiles"
  | "removeAttachment"
  | "resendFrom"
  | "compactConversation"
  | "send"
  | "continueProviderIncident"
  | "steerQueued"
  | "removeQueued"
  | "setApprovalPolicy"
  | "cycleApprovalPolicy"
  | "setCollaborationMode"
  | "decidePlan"
  | "setOutputStyle"
  | "toggleTerminal"
  | "setTerminalOpen"
  | "openProjectTerminal"
  | "setMcpOpen"
  | "setSshOpen"
  | "setSettingsOpen"
  | "setPaletteOpen"
  | "togglePalette"
  | "toggleSidebar"
  | "setSidebarCollapsed"
  | "cancelActive"
  | "resolvePermission"
  | "askSideQuestion"
  | "dismissSideQuestion"
>;

export function createInteractionActions(set: SessionSet, get: SessionGet): InteractionActions {
  return {
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
      effectiveModel = normalizeCodingModel(model !== undefined ? model : ov.model);
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
      const nextModel = normalizeCodingModel(model !== undefined ? model : localSettings.model);
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

  resendFrom: async (timelineIndex, text, skills = []) => {
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
      }, prefix);

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
        await bridge.prompt(
          session.id,
          [{ type: "text", text }, ...skills],
          uploads,
        );
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

  compactConversation: async () => {
    const { attachments, bridge, session, snapshot } = get();
    if (!bridge || !session) return;
    if (session.provider !== "local" || !bridge.compact) {
      get().flashNotice("Context compaction is available only in Clark Code conversations.");
      return;
    }
    if (attachments.length > 0) {
      get().flashNotice("Remove attachments before compacting this conversation.");
      return;
    }
    if (isBusy(snapshot)) {
      get().flashNotice("Wait for Clark to finish before compacting this conversation.");
      return;
    }
    try {
      await bridge.compact(session.id);
    } catch (error) {
      set({ error: String(error) });
    }
  },

  send: async (text, skills: SkillReferenceBlock[] = []) => {
    const { bridge, session, attachments, snapshot } = get();
    if (!bridge || !session) return;
    if (get().updateWaiting || get().updateApplying) {
      get().flashNotice("Clark Code is finishing active work before updating; send after it relaunches.");
      return;
    }
    if (!text.trim() && attachments.length === 0 && skills.length === 0) return;
    const uploads = attachments.map(toUpload);
    for (const a of attachments) if (a.previewUrl) URL.revokeObjectURL(a.previewUrl);
    set({ attachments: [], error: null });
    // A run is active in THIS conversation: queue by default. The queue drains
    // in order after each run settles, so a follow-up never changes the work
    // already in progress unless the user explicitly chooses "Steer" on it.
    if (isBusy(snapshot)) {
      const queuedMessage = { id: crypto.randomUUID(), text, uploads, skills };
      const entry = liveSessions.get(session.id);
      if (entry) entry.queued = [...entry.queued, queuedMessage];
      set((s) => ({ queued: [...s.queued, queuedMessage] }));
      return;
    }
    try {
      const entry = liveSessions.get(session.id);
      if (entry) entry.starting = true;
      try {
        await bridge.prompt(session.id, [{ type: "text", text }, ...skills], uploads);
      } finally {
        if (entry) entry.starting = false;
      }
    } catch (e) {
      // Surface the failure instead of silently doing nothing.
      set({ error: String(e) });
    }
  },

  continueProviderIncident: async (incidentId) => {
    const { snapshot } = get();
    const incident = snapshot.provider_incidents[incidentId];
    const incidentIndex = snapshot.timeline.findIndex(
      (item) => item.item === "provider_incident" && item.id === incidentId,
    );
    if (!incident || incidentIndex < 0 || incidentIndex !== snapshot.timeline.length - 1) {
      set({ error: "That provider incident is no longer the latest saved recovery point." });
      return;
    }
    if (incident.status !== "failed" && incident.status !== "interrupted") return;
    if (isBusy(snapshot)) {
      set({ error: "Wait for the active run to finish before continuing saved progress." });
      return;
    }
    await get().send(
      "Continue from the saved progress. Re-read current state, do not repeat completed writes, "
      + "and finish the task.",
    );
  },

  steerQueued: async (id) => {
    const { bridge, session, queued, snapshot } = get();
    const message = queued.find((candidate) => candidate.id === id);
    if (!bridge?.steer || !session || session.provider !== "local" || !message) return;
    if (message.uploads.length > 0 || message.skills.length > 0) {
      get().flashNotice("Messages with attachments or skills stay queued until Clark finishes.");
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

  };
}
