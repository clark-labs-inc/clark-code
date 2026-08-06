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
  LOCAL_ATTACHMENT_KINDS,
  attachmentKind,
  bindCloudTrajectory,
  buildResumeTranscript,
  cloudCreds,
  cloudShare,
  cloudUnshare,
  codeKeyAccountBinding,
  copyText,
  deriveTitle,
  effectiveApprovalPolicy,
  effectiveModelSettings,
  emptySnapshot,
  epochStale,
  fileToAttachment,
  isBusy,
  liveSessions,
  localConnectConfig,
  newLiveEntry,
  nextApprovalPolicy,
  nextSessionEpoch,
  normalizeCodingModel,
  normalizeReasoningEffort,
  notify,
  pickAllowOption,
  pickFolder,
  remoteTarget,
  resetFanOut,
  saveApprovalPolicy,
  saveApprovalPolicies,
  saveChatModels,
  saveCollaborationMode,
  saveLocalSettings,
  saveOutputStyle,
  scheduleCloudPut,
  snapshotBeforeTimelineItem,
  snapshotCache,
  toUpload,
  wouldAutoApprove,
} from "./sessionStore.runtime";
import { activeSpecialistContext } from "./specialistStore";
import {
  SPECIALIST_MODEL_ID,
  SPECIALIST_REASONING_EFFORT,
} from "../lib/localAgent";
import {
  scoutCartographyTarget,
  skillAdvisorTarget,
} from "../lib/specialists";
import { authAccountMatches } from "../lib/account";
import { projectDisplayName } from "../lib/projectSidebar";

const RAPID_DUPLICATE_WINDOW_MS = 750;
const modelReconfigureChains = new Map<string, Promise<void>>();

function isExplicitStopCommand(text: string): boolean {
  return /^(?:\/stop|stop|cancel|abort)[.!]?$/i.test(text.trim());
}

function queueModelReconfigure(
  id: string,
  task: () => Promise<void>,
): Promise<void> {
  const previous = modelReconfigureChains.get(id) ?? Promise.resolve();
  const entry = liveSessions.get(id);
  if (entry) entry.reconfiguring = true;

  let tracked: Promise<void>;
  tracked = previous
    .catch(() => undefined)
    .then(task)
    .finally(() => {
      if (modelReconfigureChains.get(id) !== tracked) return;
      modelReconfigureChains.delete(id);
      if (entry && liveSessions.get(id) === entry) entry.reconfiguring = false;
    });
  modelReconfigureChains.set(id, tracked);
  return tracked;
}

function drainQueuedPromptAfterReconfigure(
  id: string,
  bridge: NonNullable<SessionState["bridge"]>,
  get: SessionGet,
  set: SessionSet,
): void {
  const entry = liveSessions.get(id);
  if (
    !entry ||
    entry.reconfiguring ||
    entry.dispatching ||
    isBusy(entry.live) ||
    entry.live.pending_permission ||
    entry.queued.length === 0
  ) return;

  const [next, ...rest] = entry.queued;
  entry.dispatching = true;
  entry.queued = rest;
  if (get().session?.id === id) set({ queued: rest });
  void bridge
    .prompt(id, [{ type: "text", text: next.text }, ...next.skills], next.uploads)
    .catch((error) => {
      entry.dispatching = false;
      if (get().session?.id === id) {
        set((state) => ({
          error: String(error),
          // Same as `send`: a rejected invoke can leave the host's transient
          // `starting` flag set, so retire it here too.
          snapshot: state.snapshot.starting === true
            ? { ...state.snapshot, starting: false }
            : state.snapshot,
        }));
      }
    });
}

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
  | "setDefaultApprovalPolicy"
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
    const { session, chatModels, localSettings, auth, snapshot } = get();
    if (session?.provider === "specialist" || activeSpecialistContext()) return;
    const id = session?.id;
    const liveEntry = id ? liveSessions.get(id) : undefined;
    if (
      id &&
      session?.provider === "local" &&
      (
        isBusy(snapshot) ||
        (liveEntry ? isBusy(liveEntry.live) : false) ||
        liveEntry?.starting ||
        liveEntry?.dispatching
      )
    ) {
      get().flashNotice("Finish the current run before changing models.");
      return;
    }

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
      saveChatModels(nextChatModels, codeKeyAccountBinding(get().auth));
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
      saveLocalSettings(nextLocal, codeKeyAccountBinding(get().auth));
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
        ? localConnectConfig(
          effSettings,
          remoteTarget(activeRemote),
          scoutCartographyTarget(activeSpecialistContext(), activeRemote, get().activeRemoteHost),
          activeSpecialistContext()?.kind,
          codeKeyAccountBinding(get().auth),
          skillAdvisorTarget(activeSpecialistContext(), effSettings.advisorTrainingEnabled),
        )
        : localConnectConfig(
          effSettings,
          undefined,
          scoutCartographyTarget(activeSpecialistContext(), undefined, "local"),
          activeSpecialistContext()?.kind,
          codeKeyAccountBinding(get().auth),
          skillAdvisorTarget(activeSpecialistContext(), effSettings.advisorTrainingEnabled),
        );
      await queueModelReconfigure(session.id, () => bridge.reconfigure!(session.id, config));
      drainQueuedPromptAfterReconfigure(session.id, bridge, get, set);
    } catch (e) {
      if (!authAccountMatches(auth, get().auth)) return;
      const message = String(e);
      if (message.toLowerCase().includes("tool registry is still in use")) {
        get().flashNotice("Finish the current run before changing models.");
      } else {
        set({ error: `Model switch failed: ${message}` });
      }
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
      if (!authAccountMatches(auth, get().auth)) return;
      const copied = await copyText(url);
      if (!authAccountMatches(auth, get().auth)) return;
      get().flashNotice(
        copied ? "Share link copied — anyone with it can view this chat." : "Sharing on — link ready to copy.",
      );
      void notify("Share link copied", "Anyone with the link can view this conversation.");
    } catch (e) {
      if (!authAccountMatches(auth, get().auth)) return;
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
      if (!authAccountMatches(auth, get().auth)) return;
      get().flashNotice("Sharing stopped — the public link no longer works.");
      void notify("Sharing stopped", "The public link no longer works.");
    } catch (e) {
      if (!authAccountMatches(auth, get().auth)) return;
      set({ error: `Stopping the share failed: ${String(e)}` });
    }
  },

  addFiles: async (files) => {
    const state = get();
    const requestAuth = state.auth;
    const providerId = state.session?.provider ?? state.activeProvider ?? "local";
    const providerCapabilities = state.session?.capabilities
      ?? state.providers.find((provider) => provider.id === providerId)?.capabilities;
    const supportedKinds = new Set(
      providerCapabilities?.attachment_kinds
      ?? (providerId === "local" ? LOCAL_ATTACHMENT_KINDS : []),
    );
    const supported = files.filter((file) => supportedKinds.has(attachmentKind(file.name, file.type)));
    const unsupported = files.length - supported.length;
    const incoming = supported.filter((file) => file.size <= MAX_ATTACHMENT_BYTES);
    const tooBig = supported.length - incoming.length;
    if (unsupported > 0) {
      set({ error: `${unsupported} file(s) skipped — this provider cannot ingest that attachment type.` });
    }
    if (tooBig > 0) {
      set({ error: `${tooBig} file(s) skipped — over ${MAX_ATTACHMENT_BYTES / 1024 / 1024}MB.` });
    }
    const prepared = await Promise.all(incoming.map(fileToAttachment));
    if (!authAccountMatches(requestAuth, get().auth)) {
      for (const attachment of prepared) {
        if (attachment.previewUrl) URL.revokeObjectURL(attachment.previewUrl);
      }
      return;
    }
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
    if (!bridge || !session) return null;
    if (state.updateWaiting || state.updateApplying) {
      get().flashNotice("Clark Code is finishing active work before updating; edit after it relaunches.");
      return null;
    }
    const rejectEdit = (error: string) =>
      set({ error, composerPrefill: { text, timelineIndex } });
    if (session.provider !== "local") {
      rejectEdit("Editing earlier turns is currently available in Clark Code only.");
      return null;
    }
    if (!text.trim() && state.attachments.length === 0) return null;
    if (isBusy(snapshot)) {
      rejectEdit("Stop the current run before editing an earlier message.");
      return null;
    }
    const target = snapshot.timeline[timelineIndex];
    if (target?.item !== "message" || target.role !== "user") {
      rejectEdit("That message changed before it could be edited. Try again.");
      return null;
    }
    const previousEntry = liveSessions.get(session.id);
    if (!previousEntry) {
      rejectEdit("This conversation is no longer live. Reopen it and try again.");
      return null;
    }
    const requestAuth = state.auth;
    const operationEpoch = nextSessionEpoch();

    await get().ensureCodeKey();
    if (
      epochStale(operationEpoch)
      || !authAccountMatches(requestAuth, get().auth)
      || liveSessions.get(session.id) !== previousEntry
    ) return null;
    state = get();
    const projectRoot = previousEntry.projectRoot || state.activeProjectRoot;
    if (!projectRoot) {
      rejectEdit("This conversation has no project folder to resume from.");
      return null;
    }

    const prefix = snapshotBeforeTimelineItem(snapshot, timelineIndex);
    const historyPrefix = prefix.timeline.length > 0 ? prefix : null;
    const resume = buildResumeTranscript(prefix);
    const effective = effectiveModelSettings(state.localSettings, state.chatModels, session.id);
    const settings = { ...state.localSettings, ...effective, cwd: projectRoot };
    const previousMeta = state.conversations.find((conversation) => conversation.id === session.id);
    const config = previousEntry.remote
      ? localConnectConfig(
        settings,
        remoteTarget(previousEntry.remote),
        scoutCartographyTarget(previousMeta?.specialist, previousEntry.remote, previousEntry.remoteHost),
        previousMeta?.specialist?.kind,
        codeKeyAccountBinding(state.auth),
        skillAdvisorTarget(previousMeta?.specialist, settings.advisorTrainingEnabled),
      )
      : localConnectConfig(
        settings,
        undefined,
        scoutCartographyTarget(previousMeta?.specialist, undefined, "local"),
        previousMeta?.specialist?.kind,
        codeKeyAccountBinding(state.auth),
        skillAdvisorTarget(previousMeta?.specialist, settings.advisorTrainingEnabled),
      );
    const options: SessionOptions = {
      cwd: projectRoot,
      mode: state.approvalPolicy,
      collaboration_mode: state.collaborationMode,
      ...(resume ? { resume } : {}),
    };
    const uploads = state.attachments.map(toUpload);
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
      // Ignore the clean snapshot emitted while the replacement session is
      // registered. Otherwise it can be routed into the old live entry and
      // briefly restore the abandoned branch before this function swaps it.
      liveSessions.delete(session.id);
      detached = true;
      opened = await bridge.openSession("local", config, {
        kind: "new",
        options,
        bindId: session.id,
      });
      replaced = true;
      if (epochStale(operationEpoch) || !authAccountMatches(requestAuth, get().auth)) {
        void bridge.closeSession?.(opened.id);
        return null;
      }
      await bindCloudTrajectory(bridge, opened, meta, state.auth, {
        resumed: true,
        editedFromTimelineIndex: timelineIndex,
        projectMode: previousEntry.remote ? "remote" : "local",
        model: previousMeta?.specialist ? SPECIALIST_MODEL_ID : effective.model,
        reasoningEffort: previousMeta?.specialist
          ? SPECIALIST_REASONING_EFFORT
          : effective.reasoningEffort,
        approvalPolicy: state.approvalPolicy,
        outputStyle: state.outputStyle,
      }, prefix);
      if (epochStale(operationEpoch) || !authAccountMatches(requestAuth, get().auth)) {
        void bridge.closeSession?.(opened.id);
        return null;
      }

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
        return await bridge.prompt(
          session.id,
          [{ type: "text", text }, ...skills],
          uploads,
        );
      } finally {
        nextEntry.starting = false;
      }
    } catch (error) {
      if (epochStale(operationEpoch) || !authAccountMatches(requestAuth, get().auth)) {
        if (opened) void bridge.closeSession?.(opened.id);
        return null;
      }
      if (ready) {
        set({ error: String(error), connecting: false });
        return null;
      }
      if (detached && !replaced) liveSessions.set(session.id, previousEntry);
      if (replaced) {
        liveSessions.delete(session.id);
        if (opened) void bridge.closeSession?.(opened.id);
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
      return null;
    }
  },

  compactConversation: async () => {
    const { attachments, auth, bridge, session, snapshot } = get();
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
      if (!authAccountMatches(auth, get().auth) || get().session?.id !== session.id) return;
      set({ error: String(error) });
    }
  },

  send: async (text, skills: SkillReferenceBlock[] = []) => {
    const { auth, bridge, session, attachments, snapshot } = get();
    if (!bridge || !session) return { kind: "not_sent" };
    if (get().updateWaiting || get().updateApplying) {
      get().flashNotice("Clark Code is finishing active work before updating; send after it relaunches.");
      return { kind: "not_sent" };
    }
    if (!text.trim() && attachments.length === 0 && skills.length === 0) {
      return { kind: "not_sent" };
    }
    const entry = liveSessions.get(session.id);
    const normalizedText = text.trim();
    const activeRunId = Object.values(snapshot.runs).find(
      (run) => run.status === "running" || run.status === "queued",
    )?.id;
    if (
      activeRunId &&
      attachments.length === 0 &&
      skills.length === 0 &&
      isExplicitStopCommand(normalizedText)
    ) {
      set({ error: null });
      try {
        await bridge.cancel(session.id, activeRunId);
        return { kind: "cancelled" };
      } catch (error) {
        set({ error: `Stopping Clark failed: ${String(error)}` });
        return { kind: "not_sent" };
      }
    }
    const rapidDuplicate =
      entry &&
      attachments.length === 0 &&
      skills.length === 0 &&
      entry.lastSubmittedText === normalizedText &&
      Date.now() - entry.lastSubmittedAt < RAPID_DUPLICATE_WINDOW_MS;
    if (rapidDuplicate) return { kind: "not_sent" };
    if (entry && attachments.length === 0 && skills.length === 0) {
      entry.lastSubmittedText = normalizedText;
      entry.lastSubmittedAt = Date.now();
    }
    const uploads = attachments.map(toUpload);
    for (const a of attachments) if (a.previewUrl) URL.revokeObjectURL(a.previewUrl);
    set({ attachments: [], error: null });
    // A run is active in THIS conversation: queue by default. The queue drains
    // in order after each run settles, so a follow-up never changes the work
    // already in progress unless the user explicitly chooses "Steer" on it.
    if (
      isBusy(snapshot) ||
      entry?.starting ||
      entry?.dispatching ||
      entry?.reconfiguring
    ) {
      const queuedMessage = { id: crypto.randomUUID(), text, uploads, skills };
      if (entry) entry.queued = [...entry.queued, queuedMessage];
      set((s) => ({ queued: [...s.queued, queuedMessage] }));
      return { kind: "queued", queueId: queuedMessage.id };
    }
    const optimisticRun = text.trim() ? `optimistic-user-${crypto.randomUUID()}` : null;
    if (optimisticRun) {
      set((state) => ({
        snapshot: {
          ...state.snapshot,
          timeline: [
            ...state.snapshot.timeline,
            {
              item: "message",
              run: optimisticRun,
              role: "user",
              blocks: [{ type: "text", text }],
            },
          ],
        },
      }));
    }
    try {
      if (entry) entry.starting = true;
      try {
        const receipt = await bridge.prompt(
          session.id,
          [{ type: "text", text }, ...skills],
          uploads,
        );
        return { kind: "started", receipt };
      } finally {
        if (entry) entry.starting = false;
      }
    } catch (e) {
      if (entry && entry.lastSubmittedText === normalizedText) {
        entry.lastSubmittedText = null;
        entry.lastSubmittedAt = 0;
      }
      // Surface the failure instead of silently doing nothing.
      if (!authAccountMatches(auth, get().auth) || get().session?.id !== session.id) {
        return { kind: "not_sent" };
      }
      set((state) => ({
        error: String(e),
        composerPrefill: { text },
        snapshot: {
          ...state.snapshot,
          // A rejected invoke can leave the transient `starting` flag set if
          // the host's own rejection clear (or RunStarted) never landed here;
          // retire it so the activity row doesn't stay animated under the error.
          starting: state.snapshot.starting === true ? false : state.snapshot.starting,
          timeline: state.snapshot.timeline.filter(
            (item) => !optimisticRun || !(item.item === "message" && item.run === optimisticRun),
          ),
        },
      }));
      return { kind: "not_sent" };
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
    const { auth, bridge, session, queued, snapshot } = get();
    const message = queued.find((candidate) => candidate.id === id);
    if (!bridge?.steer || !session || session.provider !== "local" || !message) return;
    if (message.uploads.length > 0 || message.skills.length > 0) {
      get().flashNotice("Messages with attachments or skills stay queued until Clark finishes.");
      return;
    }
    if (!isBusy(snapshot)) return;
    const activeRunId = Object.values(snapshot.runs).find(
      (run) => run.status === "running" || run.status === "queued",
    )?.id;
    const stopping = Boolean(activeRunId && isExplicitStopCommand(message.text));
    try {
      if (activeRunId && stopping) {
        await bridge.cancel(session.id, activeRunId);
      } else {
        await bridge.steer(session.id, [{ type: "text", text: message.text }]);
      }
      if (!authAccountMatches(auth, get().auth) || get().session?.id !== session.id) return;
      get().removeQueued(id);
    } catch (error) {
      // The run may have settled between the click and the native command. Keep
      // the message safely queued; the normal drain will send it next.
      if (authAccountMatches(auth, get().auth) && get().session?.id === session.id) {
        if (stopping) {
          set({ error: `Stopping Clark failed: ${String(error)}` });
        } else {
          get().flashNotice("Clark finished before the message could steer; it remains queued.");
        }
      }
    }
  },

  removeQueued: (id) => {
    const session = get().session;
    const entry = session ? liveSessions.get(session.id) : undefined;
    if (entry) entry.queued = entry.queued.filter((q) => q.id !== id);
    set((s) => ({ queued: s.queued.filter((q) => q.id !== id) }));
  },

  setApprovalPolicy: (mode) => {
    const { auth, bridge, session, approvalPolicies } = get();
    // With an open local chat, change THAT chat's level — not every live
    // conversation's. Other chats keep running under whatever they were pinned
    // with; only the focused conversation's override (and its host mode) move.
    // With no open chat (the start screen) edit the global default instead; the
    // next chat pins it for itself when it goes live.
    const hasOpenLocal = session?.provider === "local";
    if (hasOpenLocal && session) {
      const id = session.id;
      const nextPolicies = { ...approvalPolicies, [id]: mode };
      saveApprovalPolicies(nextPolicies, codeKeyAccountBinding(auth));
      // Keep the live pool's copy in sync so the background drain and a reattach
      // see the new mode.
      const entry = liveSessions.get(id);
      if (entry && entry.session.provider === "local") {
        entry.session = { ...entry.session, mode };
      }
      set({ approvalPolicies: nextPolicies, session: { ...session, mode } });
      if (bridge?.setMode) {
        void bridge.setMode(id, mode).catch((error) => {
          if (authAccountMatches(auth, get().auth)) set({ error: String(error) });
        });
      }
      const { snapshot } = get();
      // If a prompt is open in this chat and the new mode would grant it,
      // resolve it now.
      const pend = snapshot.pending_permission;
      if (bridge && pend && wouldAutoApprove(mode, pend)) {
        const opt = pickAllowOption(pend);
        if (opt) {
          void bridge
            .respond(id, { kind: "permission", request: pend.id, option: opt.id })
            .catch((e) => {
              if (authAccountMatches(auth, get().auth)) set({ error: String(e) });
            });
        }
      }
    } else {
      saveApprovalPolicy(mode, codeKeyAccountBinding(auth));
      set({ approvalPolicy: mode });
    }
  },

  setDefaultApprovalPolicy: (mode) => {
    // Only the account-wide default moves — never a chat's own override and
    // never a live session's host mode. Pinned chats keep their own level, so
    // this only affects the next chat that goes live (same invariant as the
    // model default).
    saveApprovalPolicy(mode, codeKeyAccountBinding(get().auth));
    set({ approvalPolicy: mode });
  },

  cycleApprovalPolicy: () => {
    const { approvalPolicy, approvalPolicies, setApprovalPolicy, session, activeProvider } = get();
    // Permission modes only govern the local engine; with a cloud session (or
    // a cloud target on the start screen) the pill is hidden and Shift+Tab
    // cycling an invisible mode would just surprise the next local session.
    const isLocalTarget = session ? session.provider === "local" : activeProvider === "local";
    if (!isLocalTarget) return;
    // Cycle the FOCUSED chat's effective level — its own override if it has
    // one, otherwise the account's global default — so Shift+Tab changes only
    // the chat in front of you, never a sibling you can't see.
    const current = effectiveApprovalPolicy(approvalPolicy, approvalPolicies, session?.id);
    setApprovalPolicy(nextApprovalPolicy(current));
  },

  setCollaborationMode: (mode) => {
    saveCollaborationMode(mode, codeKeyAccountBinding(get().auth));
    const { auth, bridge, session } = get();
    set({
      collaborationMode: mode,
      ...(session ? { session: { ...session, collaboration_mode: mode } } : {}),
    });
    if (bridge?.setCollaborationMode && session) {
      void bridge.setCollaborationMode(session.id, mode).catch((error) => {
        if (authAccountMatches(auth, get().auth)) set({ error: String(error) });
      });
    }
  },

  decidePlan: async (planId, decision) => {
    const { auth, bridge, session } = get();
    if (!bridge || !session) return;
    try {
      await bridge.respond(session.id, {
        kind: "plan_decision",
        plan_id: planId,
        decision,
      });
      if (!authAccountMatches(auth, get().auth) || get().session?.id !== session.id) return;
      if (decision.action === "implement") {
        saveCollaborationMode("default", codeKeyAccountBinding(get().auth));
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
      if (!authAccountMatches(auth, get().auth) || get().session?.id !== session.id) return;
      set({ error: String(error) });
      throw error;
    }
  },

  setOutputStyle: (style) => {
    saveOutputStyle(style, codeKeyAccountBinding(get().auth));
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
    const requestAuth = get().auth;
    let target = path?.trim();
    if (!target) {
      try {
        target = (await pickFolder(get().localSettings.cwd || undefined))?.trim() || undefined;
      } catch (e) {
        if (!authAccountMatches(requestAuth, get().auth)) return;
        set({ error: String(e) });
        return;
      }
    }
    if (!authAccountMatches(requestAuth, get().auth)) return;
    if (!target) return;
    const cwd = target;
    // Move the composer to the picked project: detach a live session bound to a
    // different root (it keeps running in the sidebar pool). Without this the
    // composer stays pinned to the old session's activeProjectRoot and the next
    // message would still go to the previous project.
    const { session: live, runningIds, activeProjectRoot } = get();
    const previousRoot = activeProjectRoot?.trim() ?? "";
    const previousRunning = Boolean(live && runningIds.includes(live.id));
    if (previousRoot && previousRoot !== cwd) {
      get().endSession();
      if (previousRunning) {
        get().flashNotice(
          `Composer moved to ${projectDisplayName(cwd)}. ${projectDisplayName(previousRoot)} is still running in the sidebar.`,
        );
      }
    }
    get().setProjectFolder(cwd);
    // Record a launch for the picked project (a terminal that is already open
    // adds a fresh tab rooted here), but do NOT force the terminal drawer open:
    // opening a project and opening a terminal are separate actions.
    set((s) => ({
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
    if (!runId) return;
    set({ error: null });
    try {
      await bridge.cancel(session.id, runId);
    } catch (error) {
      set({ error: `Stopping Clark failed: ${String(error)}` });
    }
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
