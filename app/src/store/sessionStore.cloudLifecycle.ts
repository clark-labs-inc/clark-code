import { type SessionState, type SessionSet, type SessionGet, closeLiveSession,
  liveSessions, snapshotCache, nextSessionEpoch, emptySnapshot, settleRuns,
} from "./sessionStore.runtime";
import { composerDraftOwner, removeComposerDraft } from "../lib/composerDraft";
import { useSpecialistStore } from "./specialistStore";

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
    return;
  }

  set({
    runningIds: before.runningIds.filter((id) => id !== conversationId),
    warning: wasLive
      ? "A conversation received a newer cloud revision, so Clark Code stopped its stale local session."
      : before.warning,
  });
}
