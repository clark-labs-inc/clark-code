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
  clearUnseenFinished,
  closeLiveSession,
  codeKeyAccountBinding,
  cloudCreds,
  conversationProjectRoot,
  effectiveApprovalPolicy,
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
  pinApprovalPolicy,
  pinChatModel,
  remoteTarget,
  resetFanOut,
  scheduleCloudPut,
  snapshotCache,
  syncFanOut,
} from "./sessionStore.runtime";
import { saveManagedWorktreeBase } from "../lib/managedWorktreeSettings";
import {
  composerDraftRef,
  composerDraftOwner,
  loadComposerDraft,
  moveComposerDraft,
  saveComposerDraft,
  specialistStartComposerDraftId,
} from "../lib/composerDraft";
import { clearCloudComposerDraft } from "../lib/cloudComposerDraft";
import { saveSshHosts } from "../lib/sshHosts";
import { specialistModelSettings } from "../lib/specialistModel";
import { createSidebarConversationActions } from "./sessionStore.sidebarConversationActions";
import { activeSpecialistContext, useSpecialistStore } from "./specialistStore";
import {
  researchRuntimeSpecialist,
  productSpecialistTarget,
  type RsiScoutContextSnapshot,
  scoutCartographyTarget,
  specialistConnectConfig,
  newScoutRunRequestId,
} from "../lib/specialists";
import {
  specialistQuery,
  type ScoutSnapshotEntry,
  type ScoutWorkspace,
} from "../lib/specialistCloud";
import { authAccountMatches } from "../lib/account";
import { isQuickChatProject } from "../lib/projectSidebar";
import { productModule } from "../product/productModule";

type ConversationActions = Pick<
  SessionState,
  | "startBlockedReason"
  | "startSession"
  | "startQuickChat"
  | "setManagedWorktreeBase"
  | "confirmManagedWorktreeStart"
  | "dismissManagedWorktreeStart"
  | "endSession"
  | "openConversation"
  | "cleanupUnavailableConversation"
  | "archiveConversation"
  | "restoreConversation"
  | "deleteConversation"
  | "renameConversation"
  | "toggleConversationSelection"
  | "setConversationSelection"
  | "archiveSelectedConversations"
  | "deleteSelectedConversations"
>;

const RSI_SCOUT_CONTEXT_BUDGET_MS = 3_000;

export async function withinOptionalContextBudget<T>(
  operation: Promise<T>,
  budgetMs = RSI_SCOUT_CONTEXT_BUDGET_MS,
): Promise<T> {
  let timeout: ReturnType<typeof setTimeout> | null = null;
  try {
    return await Promise.race([
      operation,
      new Promise<T>((_, reject) => {
        timeout = setTimeout(
          () => reject(new Error("optional specialist context timed out")),
          budgetMs,
        );
      }),
    ]);
  } finally {
    if (timeout) clearTimeout(timeout);
  }
}

function syncNextSessionTarget(
  get: SessionGet,
  sessionId: string,
  provider: string,
  projectRoot: string | null,
  remoteHost: string | null,
): void {
  if (provider !== "local" && provider !== "specialist") return;
  if (remoteHost) {
    get().setProjectMode("remote");
    const accountScope = codeKeyAccountBinding(get().auth);
    const hosts = loadSshHosts(accountScope);
    const selected = hosts.find((host) => host.host.trim() === remoteHost.trim());
    if (!selected) return;
    if (projectRoot && selected.remoteRoot.trim() !== projectRoot.trim()) {
      saveSshHosts(
        hosts.map((host) => host.id === selected.id
          ? { ...host, remoteRoot: projectRoot.trim() }
          : host),
        accountScope,
      );
    }
    get().setSelectedHostId(selected.id);
    return;
  }

  get().setProjectMode("local");
  if (projectRoot && !isQuickChatProject(projectRoot, sessionId)) {
    get().setProjectFolder(projectRoot);
  }
}

export function createConversationActions(set: SessionSet, get: SessionGet): ConversationActions {
  return {
  startBlockedReason: () => {
    const { activeProvider, projectMode, localSettings, selectedHostId } = get();
    const specialist = researchRuntimeSpecialist(activeSpecialistContext());
    if (specialist && activeProvider !== "local") {
      return `${specialist.label} runs through the local Clark Code environment.`;
    }
    // Non-local providers (cloud) need no local folder/host — always ready.
    if (activeProvider !== "local") return null;
    if (projectMode === "remote") {
      const host = loadSshHosts(codeKeyAccountBinding(get().auth)).find((h) => h.id === selectedHostId);
      if (!host) return "Add a remote host.";
      if (!hostReady(host)) return "This host needs an SSH destination and remote folder.";
      return null;
    }
    return localSettingsReady(localSettings);
  },

  setManagedWorktreeBase: (base) => {
    const specialistKind = activeSpecialistContext()?.kind;
    saveManagedWorktreeBase(
      base,
      codeKeyAccountBinding(get().auth),
      get().localSettings.cwd,
      specialistKind,
    );
    set({ managedWorktreeBase: base });
  },

  confirmManagedWorktreeStart: async () => {
    const { bridge, worktreeTransition, managedWorktreeBase } = get();
    if (!bridge?.createManagedWorktree || !worktreeTransition) return;
    const transition = worktreeTransition;
    const transitionEpoch = nextSessionEpoch();
    // An explicit isolated choice supersedes any earlier "keep working here"
    // acknowledgement for the same dirty checkout.
    set({ dirtyWorktreeApproval: null });
    if (
      transition.action !== "create_isolated"
      && transition.action !== "preserve_changes"
    ) {
      set({
        error: "This branch transition needs a different checkout choice before starting a session.",
      });
      return;
    }
    const targetBranch = transition.action === "preserve_changes"
      ? transition.targetBranch ?? null
      : null;
    if (transition.action === "preserve_changes" && !targetBranch) {
      set({ error: "The requested branch is no longer available for an isolated continuation." });
      return;
    }
    set({ worktreePreparing: true, error: null });
    try {
      const created = await bridge.createManagedWorktree(transition.sourceRoot, {
        base: managedWorktreeBase,
        targetBranch,
      });
      // A folder/host change or a second confirmation may have superseded
      // this slow Git operation. Never attach an old checkout to the new
      // composer; remove the just-created, clean managed entry instead.
      if (epochStale(transitionEpoch) || get().worktreeTransition !== transition) {
        if (bridge.cleanupManagedWorktree) {
          await bridge.cleanupManagedWorktree(transition.sourceRoot, created.id);
        }
        return;
      }
      const baseLabel = transition.baseOptions.find((option) => option.id === managedWorktreeBase)?.label
        ?? "the selected base";
      set({
        pendingManagedWorktreePath: created.path,
        worktreeTransition: null,
        worktreePreparing: false,
        notice: transition.action === "preserve_changes"
          ? `Started an isolated continuation from ${baseLabel}. Your source changes remain in ${transition.sourceRoot}.`
          : `Started an isolated chat from ${baseLabel}. Your source changes remain in ${transition.sourceRoot}.`,
      });
      await get().startSession();
    } catch (error) {
      if (epochStale(transitionEpoch)) return;
      set({ error: String(error), worktreePreparing: false });
    }
  },

  dismissManagedWorktreeStart: () => {
    const transition = get().worktreeTransition;
    const resumeStart = transition?.action === "create_isolated";
    set({
      worktreeTransition: null,
      worktreePreparing: false,
      dirtyWorktreeApproval: transition && !transition.sourceIsManaged
        ? {
            sourceRoot: transition.sourceRoot,
            sourceBranch: transition.sourceBranch,
            sourceRevision: transition.sourceRevision,
            sourceChanges: transition.sourceChanges,
          }
        : null,
      notice: transition && !transition.sourceIsManaged
        ? resumeStart
          ? "Starting this chat in the current checkout."
          : "Branch change cancelled. Your current checkout is unchanged."
        : null,
    });
    // This dialog interrupted an already-requested start. Continue that start
    // immediately instead of making the user submit the composer a second time.
    // Branch-picker transitions are different: dismissing them only cancels the
    // requested branch change.
    if (resumeStart) void get().startSession();
  },

  startQuickChat: async () => {
    const bridge = get().bridge;
    if (!bridge?.prepareQuickChatWorkspace) {
      set({ error: "Quick Chat requires Clark Code native workspace." });
      return;
    }
    let quickChat;
    try {
      quickChat = await bridge.prepareQuickChatWorkspace();
    } catch (error) {
      set({ error: String(error) });
      return;
    }
    useSpecialistStore.getState().close();
    get().endSession();
    set({
      activeProvider: "local",
      projectMode: "local",
      error: null,
      pendingManagedWorktreePath: null,
      deferredSessionStartDraft: null,
      worktreeTransition: null,
      dirtyWorktreeApproval: null,
    });
    await get().startSession({ quickChat });
  },

  startSession: async (startOptions) => {
    const { bridge, activeProvider, auth } = get();
    if (!bridge || !activeProvider) return;
    const quickChat = startOptions?.quickChat ?? null;
    const epoch = nextSessionEpoch();
    let specialistContext = quickChat ? null : activeSpecialistContext();
    const specialistDefinition = researchRuntimeSpecialist(specialistContext);
    if (specialistDefinition && !specialistContext?.organizationId?.trim()) {
      set({
        error: `Join or create a Clark workspace before starting ${specialistDefinition.label}.`,
      });
      return;
    }
    // Validate the human-selected Scout authority before allocating any local
    // conversation workspace. This keeps a failed start entirely side-effect
    // free and prevents the previous package from becoming an implicit scope.
    if (specialistContext?.kind === "scout") {
      if (!specialistContext.organizationId?.trim()) {
        set({ error: "Pick or create a Scout workspace before starting." });
        return;
      }
      if (!specialistContext.workspaceId?.trim()) {
        set({ error: "Choose or create a Scout workspace before starting Scout." });
        return;
      }
      specialistContext = {
        ...specialistContext,
        scoutRunRequestId: newScoutRunRequestId(),
      };
    }
    const workspacePolicy = productModule().specialistWorkspace;
    let specialistWorkspace: { id: string; path: string } | null = null;
    if (specialistContext && workspacePolicy?.isConversationBound(specialistContext.kind)) {
      if (activeProvider !== "local") {
        set({ error: `${specialistContext.kind} runs through the local Clark Code environment.` });
        return;
      }
      try {
        specialistWorkspace = await bridge.prepareQuickChatWorkspace?.() ?? null;
      } catch (error) {
        set({ error: String(error) });
        return;
      }
      if (!specialistWorkspace) {
        set({ error: `${specialistContext.kind} requires Clark Code native workspace.` });
        return;
      }
    }
    if (specialistDefinition && activeProvider !== "local") {
      set({
        error: `${specialistDefinition.label} runs through the local Clark Code environment.`,
      });
      return;
    }
    const sessionProvider = specialistDefinition ? "specialist" : activeProvider;
    const isLocal = activeProvider === "local";
    // Scout is an organization/workspace coordinator, never a project worker.
    // A globally selected remote project must not redirect Scout into the
    // opaque coding-worker path, which cannot carry cartography authority.
    // Remote machines are discovered and authorized as explicit Scout targets
    // after the human starts the run from this neutral local workspace.
    const isRemote = isLocal
      && get().projectMode === "remote"
      && specialistContext?.kind !== "scout";
    const startHost = isRemote
      ? (loadSshHosts(codeKeyAccountBinding(get().auth)).find((h) => h.id === get().selectedHostId)?.host.trim() ?? null)
      : null;
    set({
      connecting: true,
      error: null,
      unavailableConversation: null,
      unavailableCleanupId: null,
      opening: {
        id: null,
        kind: "start",
        title: quickChat ? "New Quick Chat" : "New session",
        remoteHost: startHost,
      },
    });
    let remote: RemoteInfo | null = null;
    let nativeSession: Session | null = null;
    try {
      // Make sure an Clark Code key has been minted before the local provider
      // needs it (covers the case where sign-in's background provision is still
      // in flight or failed).
      if (isLocal) await get().ensureCodeKey();
      const localSettings = get().localSettings;
      let localSessionPath = quickChat?.path ?? specialistWorkspace?.path ?? localSettings.cwd.trim();
      if (isLocal && !isRemote && !specialistDefinition && !quickChat && !specialistWorkspace) {
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
                "This checkout needs a safe branch transition before the agent can start an isolated session.",
              );
            }
            const approval = get().dirtyWorktreeApproval;
            const approvalMatches = Boolean(
              approval
              && approval.sourceRoot === plan.sourceRoot
              && approval.sourceBranch === plan.sourceBranch
              && approval.sourceRevision === plan.sourceRevision
              && approval.sourceChanges.changedFiles === plan.sourceChanges.changedFiles
              && approval.sourceChanges.untrackedFiles === plan.sourceChanges.untrackedFiles
              && approval.sourceChanges.conflictedFiles === plan.sourceChanges.conflictedFiles,
            );
            if (!plan.sourceIsManaged && plan.requiresConfirmation && !approvalMatches) {
              set({
                connecting: false,
                opening: null,
                worktreeTransition: plan,
                worktreePreparing: false,
                dirtyWorktreeApproval: null,
                deferredSessionStartDraft: startOptions?.submittedDraft
                  ? {
                      owner: composerDraftOwner(auth?.user ?? null),
                      text: startOptions.submittedDraft,
                    }
                  : get().deferredSessionStartDraft,
              });
              return;
            }
            if (approvalMatches) set({ dirtyWorktreeApproval: null });
            // The selected checkout is the default workspace. Only create a
            // sibling worktree when the user explicitly chose the default
            // branch as the isolated starting point; selecting a project must
            // otherwise keep its checkout path stable.
            if (
              !plan.sourceIsManaged
              && !approvalMatches
              && get().managedWorktreeBase === "default"
            ) {
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

      // Remote: attach the native durable worker, then bind this conversation to
      // run its tools there. Local: run the loop on this machine. Other
      // providers connect with the signed-in the agent config, no embedded creds.
      let config;
      let options;
      let remoteHost: string | null = null;
      const collaboration_mode = get().collaborationMode;
      const mode = get().approvalPolicy;
      if (specialistDefinition && specialistContext) {
        const specialistHost = isRemote
          ? loadSshHosts(codeKeyAccountBinding(get().auth)).find((h) => h.id === get().selectedHostId)
          : undefined;
        if (isRemote && !specialistHost) {
          throw new Error("Pick a remote host first, or add one.");
        }
        if (specialistHost) {
          localSessionPath = specialistHost.remoteRoot.trim();
          remoteHost = specialistHost.host.trim();
        }
        let scoutContext: RsiScoutContextSnapshot | undefined;
        if (specialistDefinition.kind === "rsi" && specialistContext.organizationId) {
          const credentials = cloudCreds(auth);
          if (credentials) {
            const organizationId = specialistContext.organizationId;
            const preferredWorkspaceId = specialistContext.workspaceId;
            try {
              scoutContext = await withinOptionalContextBudget((async () => {
                const workspaces = await specialistQuery<ScoutWorkspace[]>(
                  credentials,
                  "scout",
                  "scout_workspaces",
                  organizationId,
                );
                const workspace = workspaces.find(
                  ({ id }) => id === preferredWorkspaceId,
                ) ?? workspaces[0];
                if (!workspace) return undefined;
                const snapshot = await specialistQuery<{ entries: ScoutSnapshotEntry[] }>(
                  credentials,
                  "scout",
                  "scout_snapshot",
                  organizationId,
                  workspace.id,
                );
                const entries = snapshot.entries.slice(0, 64).map((entry) => ({
                  objectKind: entry.object_kind,
                  objectId: entry.object_id,
                  classification: entry.event.classification,
                  attributes: entry.event.fact.attributes,
                }));
                while (
                  entries.length > 0
                  && new TextEncoder().encode(JSON.stringify({
                    schemaVersion: 1,
                    workspaceId: workspace.id,
                    entries,
                  })).length > 16 * 1024
                ) {
                  entries.pop();
                }
                return {
                  schemaVersion: 1,
                  workspaceId: workspace.id,
                  entries,
                };
              })());
            } catch {
              // RSI remains available with project-local context when Scout has
              // no workspace, the read-only snapshot is temporarily offline,
              // or optional cloud context exceeds the bounded startup budget.
            }
          }
        }
        config = specialistConnectConfig(
          specialistContext,
          localSessionPath,
          scoutContext,
          specialistHost
            ? { host: specialistHost.host.trim(), remoteRoot: localSessionPath }
            : undefined,
          localSettings.advisorTrainingEnabled === true,
        );
        options = { cwd: localSessionPath, collaboration_mode: "default" as const };
      } else if (isRemote) {
        const host = loadSshHosts(codeKeyAccountBinding(get().auth)).find((h) => h.id === get().selectedHostId);
        if (!host) throw new Error("Pick a remote host first, or add one.");
        remote = await openRemote(host, localSettings);
        remoteHost = host.host.trim();
        config = localConnectConfig(
          localSettings,
          remoteTarget(remote),
          scoutCartographyTarget(specialistContext, remote, remoteHost),
          specialistContext?.kind,
          codeKeyAccountBinding(get().auth),
          productSpecialistTarget(specialistContext, localSettings.advisorTrainingEnabled),
          specialistModelSettings(specialistContext) ?? undefined,
          get().recentProjects,
        );
        options = { cwd: remote.cwd, mode, collaboration_mode };
      } else if (isLocal) {
        const sessionSettings = { ...localSettings, cwd: localSessionPath };
        config = localConnectConfig(
          sessionSettings,
          undefined,
          scoutCartographyTarget(specialistContext, undefined, "local"),
          specialistContext?.kind,
          codeKeyAccountBinding(get().auth),
          productSpecialistTarget(specialistContext, localSettings.advisorTrainingEnabled),
          specialistModelSettings(specialistContext) ?? undefined,
          get().recentProjects,
        );
        options = { cwd: localSessionPath, mode, collaboration_mode };
      } else {
        config = {};
        options = {};
      }

      // Superseded (cancel / another open) while connecting → abandon quietly.
      if (epochStale(epoch)) {
        return;
      }

      const requestedSessionId = quickChat?.id ?? specialistWorkspace?.id;
      const session = await bridge.openSession(sessionProvider, config, {
        kind: "new",
        options,
        ...(requestedSessionId ? { bindId: requestedSessionId } : {}),
      });
      nativeSession = session;
      if (epochStale(epoch)) {
        void bridge.closeSession?.(session.id);
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
        provider: sessionProvider,
        project: projectRoot ?? project,
        remoteHost: remoteHost ?? undefined,
        mode: session.mode,
        createdAt: now,
        updatedAt: now,
        specialist: specialistContext ?? undefined,
      };
      if (sessionProvider === "local" && !specialistContext) {
        pinChatModel(get, set, session.id, localSettings);
        // Pin this new chat to the approval level it was created with, so a
        // later change to the global default never silently rewrites what an
        // already-running chat executes under (same invariant as the model).
        pinApprovalPolicy(get, set, session.id, mode);
      }
      await bindCloudTrajectory(
        bridge,
        session,
        conversationMeta,
        get().auth,
        {
          projectMode: get().projectMode,
          model: specialistContext
            ? specialistModelSettings(specialistContext)?.model
            : sessionProvider === "local"
              ? localSettings.model
              : undefined,
          reasoningEffort: specialistContext
            ? specialistModelSettings(specialistContext)?.reasoningEffort
            : sessionProvider === "local"
              ? localSettings.reasoningEffort
              : undefined,
          approvalPolicy: get().approvalPolicy,
          outputStyle: get().outputStyle,
          memoriesEnabled: get().memoriesEnabled,
          browserEnabled: get().browserEnabled,
        },
        emptySnapshot(),
      );
      if (epochStale(epoch)) {
        void bridge.closeSession?.(session.id);
        return;
      }
      if (isLocal && !isRemote && !quickChat && !specialistWorkspace && localSettings.cwd.trim()) {
        set({
          recentProjects: addRecentProject(
            localSettings.cwd.trim(),
            codeKeyAccountBinding(get().auth),
          ),
        });
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
      const deferredSessionStartDraft = get().deferredSessionStartDraft;
      set({
        session,
        snapshot: emptySnapshot(),
        connecting: false,
        opening: null,
        unavailableConversation: null,
        unavailableCleanupId: null,
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
        deferredSessionStartDraft: null,
        worktreePreparing: false,
        worktreeTransition: null,
      });
      // Only an explicitly deferred first prompt may cross the ownership
      // boundary after a dirty-checkout decision. A different draft can be a
      // newer edit from another device (or an old New session draft); moving it
      // here leaks that text into the new conversation. The equality check is
      // the local CAS: if the New session draft changed while the decision was
      // open, preserve it under New session and start with an empty composer.
      if (
        isLocal
        && !isRemote
        && !quickChat
        && !specialistContext
        && deferredSessionStartDraft
        && deferredSessionStartDraft.owner === composerDraftOwner(get().auth?.user ?? null)
        && loadComposerDraft(deferredSessionStartDraft.owner, null) === deferredSessionStartDraft.text
      ) {
        moveComposerDraft(
          deferredSessionStartDraft.owner,
          null,
          session.id,
          deferredSessionStartDraft.text,
        );
      }
    } catch (e) {
      // Brought up a tunnel but failed afterward → tear it back down.
      if (nativeSession) void bridge.closeSession?.(nativeSession.id);
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
    const abandonedSpecialistStart = get().session
      ? null
      : activeSpecialistContext()?.kind;
    if (abandonedSpecialistStart) {
      const owner = composerDraftOwner(get().auth?.user ?? null);
      const draftId = specialistStartComposerDraftId(abandonedSpecialistStart);
      saveComposerDraft(owner, draftId, "");
      const creds = cloudCreds(get().auth);
      if (creds) void clearCloudComposerDraft(creds, draftId).catch(() => {});
    }
    useSpecialistStore.getState().close();
    for (const a of get().attachments) if (a.previewUrl) URL.revokeObjectURL(a.previewUrl);
    if (opts?.force) {
      // Sign-out: tear down every live session for real.
      const bridge = get().bridge;
      for (const id of [...liveSessions.keys()]) closeLiveSession(bridge, id);
      set({ runningIds: [] });
    }
    // `useComposerDraftState` already persists text under the exact
    // conversation (or specialist start-screen) key. A new session must
    // hydrate its own key, never a global prefill copied from the surface the
    // user just left. The old draft remains recoverable when that conversation
    // is reopened.
    composerDraftRef.current = "";
    set({
      session: null,
      snapshot: emptySnapshot(),
      error: null,
      connecting: false,
      attachments: [],
      historyPrefix: null,
      opening: null,
      unavailableConversation: null,
      unavailableCleanupId: null,
      composerPrefill: null,
      queued: [],
      terminalOpen: false,
      sideQuestion: null,
      activeRemote: null,
      activeRemoteHost: null,
      activeProjectRoot: null,
      deferredSessionStartDraft: null,
      worktreeTransition: null,
      dirtyWorktreeApproval: null,
      worktreePreparing: false,
      selectedConversationIds: new Set(),
    });
  },

  openConversation: async (id) => {
    // Opening is the visit: drop any finished-but-unvisited marker for this row
    // immediately, even when the chat is already open or an open is in flight.
    if (get().unseenWorkIds.includes(id)) {
      set({ unseenWorkIds: clearUnseenFinished(get().unseenWorkIds, id) });
    }
    const { bridge, activeProvider, auth, session, providers, localSettings } = get();
    if (!bridge || !activeProvider) return;
    const targetMeta = get().conversations.find((conversation) => conversation.id === id);
    // Already opening this one (double-click, impatient re-click) → no-op; the
    // in-flight open keeps its spinner.
    if (get().opening?.id === id) return;
    if (session?.id === id) {
      if (targetMeta?.specialist) {
        useSpecialistStore.getState().open(targetMeta.specialist.kind, targetMeta.specialist);
      } else {
        useSpecialistStore.getState().close();
      }
      return;
    }
    const targetProvider = targetMeta?.provider || activeProvider;
    const showTargetSpecialist = () => {
      if (targetMeta?.specialist) {
        useSpecialistStore.getState().open(targetMeta.specialist.kind, targetMeta.specialist);
      } else {
        useSpecialistStore.getState().close();
      }
    };
    // A prefill is staging for the composer that requested it, never navigation
    // state. Clear it before changing targets so a failed send / edit retry from
    // the previous surface cannot hydrate the conversation being opened. The
    // user's actual text is already persisted under that surface's exact draft
    // key and remains there when they return.
    composerDraftRef.current = "";
    set({ composerPrefill: null });
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
        // Pin the approval level this live session is actually running under, so
        // a later global-default change can't drift a background chat. Idempotent.
        const liveMode = entry.session.mode;
        if (liveMode === "ask" || liveMode === "auto" || liveMode === "full") {
          pinApprovalPolicy(get, set, id, liveMode);
        }
      }
      for (const a of get().attachments) if (a.previewUrl) URL.revokeObjectURL(a.previewUrl);
      const merged = mergedOf(entry);
      const projectRoot = liveProjectRoot(entry.session, entry.projectRoot);
      // Switching to an idle session emits no snapshot frame, so re-sync the
      // fan-out from the reattached snapshot instead of leaving the prior
      // conversation's swarm on screen.
      resetFanOut();
      syncFanOut(merged.fan_out);
      set({
        session: entry.session,
        activeProvider: entry.session.provider === "specialist"
          ? "local"
          : entry.session.provider,
        snapshot: merged,
        historyPrefix: entry.historyPrefix,
        activeRemote: entry.remote,
        activeRemoteHost: entry.remoteHost,
        activeProjectRoot: projectRoot,
        queued: entry.queued,
        attachments: [],
        connecting: false,
        opening: null,
        unavailableConversation: null,
        unavailableCleanupId: null,
        error: null,
        dismissedFailedRuns: [],
      });
      // Establish the target conversation id before switching specialist
      // workspaces. Otherwise the specialist store can render its new-session
      // composer for one frame and expose the wrong draft ownership boundary.
      showTargetSpecialist();
      syncNextSessionTarget(
        get,
        id,
        entry.session.provider,
        projectRoot,
        entry.remoteHost,
      );
      return;
    }

    const openingMeta = get().conversations.find((c) => c.id === id);
    const providerForPreview = providers.find((candidate) => candidate.id === targetProvider);
    const cached = snapshotCache.get(id) ?? null;
    const previewProject = openingMeta?.project ?? null;
    set({
      ...(providerForPreview
        ? {
            session: {
              id,
              provider: targetProvider,
              capabilities: providerForPreview.capabilities,
              mode: openingMeta?.mode,
              collaboration_mode: get().collaborationMode,
              environment: previewProject
                ? {
                    checkout_root: previewProject,
                    workspace_roots: [previewProject],
                    remote: Boolean(openingMeta?.remoteHost),
                  }
                : undefined,
            },
            snapshot: cached ?? emptySnapshot(),
            historyPrefix: cached,
            activeProvider: targetProvider === "specialist" ? "local" : targetProvider,
            activeRemote: null,
            activeRemoteHost: openingMeta?.remoteHost ?? null,
            activeProjectRoot: previewProject,
            queued: [],
            attachments: [],
          }
        : {}),
      connecting: true,
      error: null,
      unavailableConversation: null,
      unavailableCleanupId: null,
      dismissedFailedRuns: [],
      opening: {
        id,
        kind: "open",
        title: openingMeta?.title || "Conversation",
        remoteHost: openingMeta?.remoteHost ?? null,
      },
    });
    // The opening target (and, where available, its preview session) now owns
    // the workspace. Only after that boundary exists may the specialist lens
    // render, so it can never mount against the specialist start-draft key.
    showTargetSpecialist();
    // Cloud-first: the transcript comes from the in-memory cache or a `cloudGet`.
    const restored = await fetchSnapshot(
      id,
      auth,
      () => authAccountMatches(auth, get().auth),
    );
    if (!epochStale(epoch) && restored) {
      set({ snapshot: restored, historyPrefix: restored });
    }
    // The cloud read itself can outlive a later click or a deletion event. Stop
    // here before attaching a native worker or provider for a target the user no
    // longer has selected.
    if (epochStale(epoch)) return;
    let remote: RemoteInfo | null = null;
    let nativeSession: Session | null = null;
    try {
      const provider = providers.find((candidate) => candidate.id === targetProvider);
      if (!provider) {
        throw new Error(
          `The ${targetMeta?.provider || "saved"} provider for this conversation is no longer available.`,
        );
      }
      const isLocal = targetProvider === "local";
      const isSpecialist = targetProvider === "specialist";
      const isProjectProvider = isLocal || isSpecialist;
      const quickChat = isLocal && isQuickChatProject(openingMeta?.project, id);
      const canResume = provider.capabilities.load_session;
      if (
        isLocal
        && !canResume
        && !restored
        && openingMeta
        && bridge.configureCloudTrajectory
      ) {
        throw new Error(
          "This conversation’s saved history is unavailable, so Clark Code did not open an empty replacement.",
        );
      }

      // A remote conversation reconnects its host (matched by SSH destination);
      // the saved host must still exist on this device.
      const wantRemote = isLocal && !!openingMeta?.remoteHost;
      let requestedProjectRoot = conversationProjectRoot(
        openingMeta?.project,
        localSettings.cwd,
      );
      if (quickChat) {
        if (!bridge.prepareQuickChatWorkspace) {
          throw new Error("Quick Chat requires Clark Code native workspace.");
        }
        requestedProjectRoot = (await bridge.prepareQuickChatWorkspace(id)).path;
        const specialistKind = openingMeta?.specialist?.kind;
        const workspacePolicy = productModule().specialistWorkspace;
        if (
          specialistKind
          && workspacePolicy?.isConversationBound(specialistKind)
        ) {
          await workspacePolicy.prepareDocument?.(specialistKind, id);
        }
      }
      let config;
      let options;
      let remoteHost: string | null = null;
      // Reopened local sessions resume in the composer's collaboration mode.
      // The model comes from the conversation's per-chat override when one was
      // set, else the global default — so reopening a chat that ran a different
      // model starts it on that model again, not the current default.
      const collaboration_mode = get().collaborationMode;
      // The approval level likewise comes from this chat's own override when it
      // has one, else the global default — so reopening a chat you'd set to
      // "Full access" restarts it there, not on whatever the composer happens
      // to show now.
      const mode = effectiveApprovalPolicy(get().approvalPolicy, get().approvalPolicies, id);
      const effSettings = effectiveModelSettings(localSettings, get().chatModels, id);
      if (isLocal && !openingMeta?.specialist) {
        pinChatModel(get, set, id, effSettings);
        // Pin this chat to its (effective) approval level the first time it
        // reopens, so a later global-default change can't drift it. Idempotent:
        // a chat already pinned keeps its own level.
        pinApprovalPolicy(get, set, id, mode);
      }
      // A legacy Scout conversation without an exact workspace binding cannot
      // be repaired by silently selecting or creating cloud authority during
      // reopen. The user must start a new Scout conversation and make that
      // binding explicit.
      let resolvedSpecialist = openingMeta?.specialist;
      if (openingMeta?.specialist?.kind === "scout" && !openingMeta.specialist.workspaceId?.trim()) {
        throw new Error(
          "This Scout conversation has no workspace binding. Start a new Scout conversation and choose a workspace.",
        );
      }
      if (isSpecialist) {
        if (!openingMeta?.specialist) {
          throw new Error("This specialist conversation has no saved specialist context.");
        }
        if (!requestedProjectRoot) {
          throw new Error(
            "This specialist conversation has no project folder. Choose one before reopening it.",
          );
        }
        await get().ensureCodeKey();
        const specialistHost = wantRemote
          ? loadSshHosts(codeKeyAccountBinding(get().auth)).find(
            (host) => host.host.trim() === openingMeta?.remoteHost,
          )
          : undefined;
        if (wantRemote && !specialistHost) {
          throw new Error(`Add the SSH host "${openingMeta!.remoteHost}" to reopen this remote conversation.`);
        }
        config = specialistConnectConfig(
          openingMeta.specialist,
          requestedProjectRoot,
          undefined,
          specialistHost
            ? { host: specialistHost.host.trim(), remoteRoot: requestedProjectRoot }
            : undefined,
          get().localSettings.advisorTrainingEnabled === true,
        );
        remoteHost = specialistHost?.host.trim() ?? null;
        options = { cwd: requestedProjectRoot, collaboration_mode: "default" as const };
      } else if (wantRemote) {
        const host = loadSshHosts(codeKeyAccountBinding(get().auth)).find((h) => h.host.trim() === openingMeta!.remoteHost);
        if (!host) {
          throw new Error(`Add the SSH host "${openingMeta!.remoteHost}" to reopen this remote conversation.`);
        }
        remote = await openRemote(
          host,
          effSettings,
          conversationProjectRoot(openingMeta?.project, host.remoteRoot),
        );
        remoteHost = host.host.trim();
        config = localConnectConfig(
          effSettings,
          remoteTarget(remote),
          scoutCartographyTarget(resolvedSpecialist, remote, remoteHost),
          resolvedSpecialist?.kind,
          codeKeyAccountBinding(get().auth),
          productSpecialistTarget(resolvedSpecialist, effSettings.advisorTrainingEnabled),
          specialistModelSettings(resolvedSpecialist) ?? undefined,
          get().recentProjects,
        );
        options = { cwd: remote.cwd, mode, collaboration_mode };
      } else if (isLocal) {
        if (!quickChat && !requestedProjectRoot) {
          throw new Error("This conversation has no project folder. Choose one before reopening it.");
        }
        config = localConnectConfig(
          { ...effSettings, cwd: requestedProjectRoot },
          undefined,
          scoutCartographyTarget(resolvedSpecialist, undefined, "local"),
          resolvedSpecialist?.kind,
          codeKeyAccountBinding(get().auth),
          productSpecialistTarget(resolvedSpecialist, effSettings.advisorTrainingEnabled),
          specialistModelSettings(resolvedSpecialist) ?? undefined,
          get().recentProjects,
        );
        options = { cwd: requestedProjectRoot, mode, collaboration_mode };
      } else {
        config = {};
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
        return;
      }

      // Providers that can't resume (the local agent has no server-side session)
      // reopen as a fresh session BOUND to the conversation id (the host keys
      // the session and tags its snapshots by it), so it doesn't fork into a
      // duplicate and events route back to this conversation.
      const opened = await bridge.openSession(
        targetProvider,
        config,
        canResume
          ? { kind: "load", id }
          : { kind: "new", options, bindId: id },
      );
      nativeSession = opened;
      if (epochStale(epoch)) {
        void bridge.closeSession?.(opened.id);
        return;
      }
      const trajectoryMeta: ConversationMeta = openingMeta
        ? quickChat && opened.environment?.checkout_root
          ? { ...openingMeta, project: opened.environment.checkout_root }
          : openingMeta
        : {
        id,
        title: "Conversation",
        provider: targetProvider,
        project: isProjectProvider
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
        projectMode: wantRemote ? "remote" : quickChat ? "quick_chat" : "local",
        model: openingMeta?.specialist
          ? specialistModelSettings(openingMeta.specialist)?.model
          : isLocal
            ? effSettings.model
            : undefined,
        reasoningEffort: openingMeta?.specialist
          ? specialistModelSettings(openingMeta.specialist)?.reasoningEffort
          : isLocal
            ? effSettings.reasoningEffort
            : undefined,
        approvalPolicy: get().approvalPolicy,
        outputStyle: get().outputStyle,
      }, restored ?? emptySnapshot());
      if (epochStale(epoch)) {
        void bridge.closeSession?.(opened.id);
        return;
      }
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
        activeProvider: targetProvider === "specialist" ? "local" : targetProvider,
        historyPrefix: restored,
        snapshot: restored ?? emptySnapshot(),
        connecting: false,
        opening: null,
        unavailableConversation: null,
        unavailableCleanupId: null,
        attachments: [],
        queued: [],
        activeRemote: remote,
        activeRemoteHost: remoteHost,
        activeProjectRoot: projectRoot,
        warning: restored?.sync_pending
          ? "This conversation was recovered from local disk and will sync when product cloud is reachable."
          : get().warning,
      });
      syncNextSessionTarget(get, id, targetProvider, projectRoot, remoteHost);
    } catch (e) {
      if (nativeSession) void bridge.closeSession?.(nativeSession.id);
      if (epochStale(epoch)) return;
      // The clicked conversation remains the navigation target even though it
      // could not become a live provider session. Detach the previously shown
      // chat (its live entry keeps running in the background) so the workspace
      // cannot snap back to stale content behind a generic error toast.
      set({
        session: null,
        snapshot: emptySnapshot(),
        error: null,
        connecting: false,
        opening: null,
        unavailableConversation: {
          id,
          title: openingMeta?.title || "Conversation",
          detail: String(e),
          kind: "unavailable",
        },
        unavailableCleanupId: null,
        attachments: [],
        historyPrefix: null,
        queued: [],
        terminalOpen: false,
        sideQuestion: null,
        activeRemote: null,
        activeRemoteHost: null,
        activeProjectRoot: null,
      });
    }
  },

  cleanupUnavailableConversation: async () => {
    const unavailable = get().unavailableConversation;
    if (!unavailable || unavailable.kind !== "unavailable") return;
    const owner = composerDraftOwner(get().auth?.user ?? null);
    set({ unavailableCleanupId: unavailable.id, error: null });
    await get().deleteConversation(unavailable.id);
    // A failed durable delete keeps both the row and its recovery surface. A
    // later navigation also supersedes cleanup: never reset a different chat
    // merely because this deletion acknowledgement arrived late.
    if (get().conversations.some((conversation) => conversation.id === unavailable.id)) {
      if (get().unavailableCleanupId === unavailable.id) {
        set({ unavailableCleanupId: null });
      }
      return;
    }
    if (
      get().unavailableCleanupId !== unavailable.id
      || get().unavailableConversation?.id !== unavailable.id
    ) {
      if (get().unavailableCleanupId === unavailable.id) {
        set({ unavailableCleanupId: null });
      }
      return;
    }

    resetFanOut();
    useSpecialistStore.getState().close();
    saveComposerDraft(owner, null, "");
    const creds = cloudCreds(get().auth);
    if (creds) void clearCloudComposerDraft(creds, null).catch(() => {});
    // Cleanup is an explicit reset boundary: retain recent-project history for
    // later use, but do not pin the fresh composer to the failed chat's project.
    get().setLocalSettings({ cwd: "" });
    set({
      session: null,
      snapshot: emptySnapshot(),
      error: null,
      connecting: false,
      opening: null,
      unavailableConversation: null,
      unavailableCleanupId: null,
      attachments: [],
      historyPrefix: null,
      composerPrefill: null,
      queued: [],
      terminalOpen: false,
      sideQuestion: null,
      activeRemote: null,
      activeRemoteHost: null,
      activeProjectRoot: null,
      projectMode: "local",
      selectedConversationIds: new Set(),
    });
  },

  renameConversation: async (id, title) => {
    const requestAuth = get().auth;
    const clean = title.trim();
    const prev = get().conversations.find((c) => c.id === id);
    if (!prev || !clean || clean === prev.title) return;
    const updated = { ...prev, title: clean, titleLocked: true };
    set({ conversations: get().conversations.map((c) => (c.id === id ? updated : c)) });
    // Persist the title to the cloud. A `put` carries the whole snapshot, so
    // fetch it (cache or cloud) first — this also covers renaming a chat that
    // wasn't opened this session.
    const creds = cloudCreds(requestAuth);
    if (!creds) return;
    const snap = await fetchSnapshot(
      id,
      requestAuth,
      () => authAccountMatches(requestAuth, get().auth),
    );
    if (snap && authAccountMatches(requestAuth, get().auth)) {
      scheduleCloudPut(creds, updated, snap);
    }
  },

  ...createSidebarConversationActions(set, get),

  };
}
