import type { SidebarConversationMutationKind } from "../lib/sidebarConversationInteractions";
import { composerDraftOwner, removeComposerDraft } from "../lib/composerDraft";
import { clearCloudComposerDraft } from "../lib/cloudComposerDraft";
import { authAccountMatches } from "../lib/account";
import { useSpecialistStore } from "./specialistStore";
import {
  type SessionGet,
  type SessionSet,
  type SessionState,
  BUSY_SESSION_MESSAGE,
  closeLiveSession,
  cloudCreds,
  cloudDelete,
  cloudSetArchived,
  emptySnapshot,
  fetchSnapshot,
  isBusy,
  liveSessions,
  mergedOf,
  epochStale,
  nextSessionEpoch,
  releaseSnapshotCheckpoints,
  snapshotCache,
} from "./sessionStore.runtime";

type SidebarConversationActions = Pick<
  SessionState,
  | "archiveConversation"
  | "restoreConversation"
  | "deleteConversation"
  | "toggleConversationSelection"
  | "setConversationSelection"
  | "archiveSelectedConversations"
  | "deleteSelectedConversations"
>;

let nextMutationId = 0;
const MUTATION_SUMMARY_MS = 2200;

/** Let a confirmed row render its exit before the next durable mutation starts.
 * Two frames make the feedback visible in the browser while remaining a no-op
 * in the node test environment. */
function nextMutationPaint(): Promise<void> {
  if (typeof requestAnimationFrame !== "function") return Promise.resolve();
  return new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
}

function beginMutation(
  set: SessionSet,
  kind: SidebarConversationMutationKind,
  ids: readonly string[],
): number {
  const id = ++nextMutationId;
  set({
    conversationMutation: {
      id,
      kind,
      total: ids.length,
      completed: 0,
      failed: 0,
      pending: ids.length,
    },
    mutatingConversationIds: new Set(ids),
  });
  return id;
}

function settleMutation(
  state: SessionState,
  conversationId: string,
  mutationId: number,
  success: boolean,
) {
  const mutatingConversationIds = new Set(state.mutatingConversationIds);
  mutatingConversationIds.delete(conversationId);
  const mutation = state.conversationMutation;
  if (!mutation || mutation.id !== mutationId) return { mutatingConversationIds };
  return {
    mutatingConversationIds,
    conversationMutation: {
      ...mutation,
      pending: Math.max(0, mutation.pending - 1),
      completed: mutation.completed + (success ? 1 : 0),
      failed: mutation.failed + (success ? 0 : 1),
    },
  };
}

function clearMutationSummary(set: SessionSet, get: SessionGet, mutationId: number): void {
  setTimeout(() => {
    const mutation = get().conversationMutation;
    if (mutation?.id === mutationId && mutation.pending === 0) {
      set({ conversationMutation: null });
    }
  }, MUTATION_SUMMARY_MS);
}

function clearActiveConversation(state: SessionState, id: string) {
  const liveTarget = state.session?.id === id;
  const unavailableTarget = state.unavailableConversation?.id === id;
  const openingTarget = state.opening?.id === id;
  if (!liveTarget && !unavailableTarget && !openingTarget) return {};
  const cleanupOwnsUnavailable =
    unavailableTarget && state.unavailableCleanupId === id;
  return {
    ...(liveTarget ? { session: null, snapshot: emptySnapshot() } : {}),
    ...(cleanupOwnsUnavailable ? {} : { unavailableConversation: null }),
    ...(openingTarget ? { opening: null, connecting: false } : {}),
    error: null,
    attachments: [],
    historyPrefix: null,
    composerPrefill: null,
    queued: [],
    terminalOpen: false,
    sideQuestion: null,
    activeRemote: null,
    activeRemoteHost: null,
    activeProjectRoot: null,
  };
}

function availableTargets(
  get: SessionGet,
  ids: readonly string[],
  kind: SidebarConversationMutationKind,
): string[] {
  if (get().mutatingConversationIds.size > 0) return [];
  const requested = [...new Set(ids)];
  const existing = requested.filter((id) => {
    const conversation = get().conversations.find((candidate) => candidate.id === id);
    if (!conversation) return false;
    if (kind === "archive") return !conversation.archived;
    if (kind === "restore") return conversation.archived;
    return true;
  });
  const busy = existing.filter((id) => {
    const entry = liveSessions.get(id);
    return entry && isBusy(entry.live);
  });
  if (busy.length > 0) get().flashNotice(BUSY_SESSION_MESSAGE);
  return existing.filter((id) => !busy.includes(id));
}

async function archiveConversations(
  set: SessionSet,
  get: SessionGet,
  requestedIds: readonly string[],
): Promise<void> {
  const targets = availableTargets(get, requestedIds, "archive");
  if (targets.length === 0) return;
  const mutationId = beginMutation(set, "archive", targets);
  const requestAuth = get().auth;
  const creds = cloudCreds(requestAuth);

  // Local-only archive can otherwise begin and finish inside one React batch,
  // skipping the row spinner and active-workspace transition entirely.
  await nextMutationPaint();

  for (const id of targets) {
    if (!authAccountMatches(requestAuth, get().auth)) return;
    try {
      if (creds) await cloudSetArchived(creds, id, true);
      if (!authAccountMatches(requestAuth, get().auth)) return;
      const cancelsOpening = get().opening?.id === id;
      const clearsVisibleTarget =
        cancelsOpening
        || get().session?.id === id
        || get().unavailableConversation?.id === id;
      if (cancelsOpening) nextSessionEpoch();
      if (
        clearsVisibleTarget
        && get().unavailableCleanupId !== id
      ) {
        useSpecialistStore.getState().close();
      }
      closeLiveSession(get().bridge, id);
      set((state) => ({
        conversations: state.conversations.map((conversation) =>
          conversation.id === id ? { ...conversation, archived: true } : conversation,
        ),
        runningIds: state.runningIds.filter((runningId) => runningId !== id),
        unseenWorkIds: state.unseenWorkIds.filter((unseenId) => unseenId !== id),
        selectedConversationIds: new Set(
          [...state.selectedConversationIds].filter((selectedId) => selectedId !== id),
        ),
        ...clearActiveConversation(state, id),
        ...settleMutation(state, id, mutationId, true),
      }));
    } catch (error) {
      if (!authAccountMatches(requestAuth, get().auth)) return;
      set((state) => ({
        error: `Could not archive this conversation: ${String(error)}`,
        ...settleMutation(state, id, mutationId, false),
      }));
    }
    await nextMutationPaint();
  }
  clearMutationSummary(set, get, mutationId);
}

async function deleteConversations(
  set: SessionSet,
  get: SessionGet,
  requestedIds: readonly string[],
): Promise<void> {
  const targets = availableTargets(get, requestedIds, "delete");
  if (targets.length === 0) return;
  const mutationId = beginMutation(set, "delete", targets);
  const requestAuth = get().auth;
  const creds = cloudCreds(requestAuth);
  const draftOwner = composerDraftOwner(requestAuth?.user ?? null);

  // Give the sidebar and active workspace one committed visual state before a
  // cached snapshot makes the first durable delete complete immediately.
  await nextMutationPaint();

  for (const id of targets) {
    if (!authAccountMatches(requestAuth, get().auth)) return;
    const entry = liveSessions.get(id);
    const meta = get().conversations.find((conversation) => conversation.id === id);
    try {
      // Capture checkpoint ownership before cloudDelete tombstones the snapshot
      // pipeline. The release remains best-effort, just as it was before, but
      // never holds the visible deletion hostage after the durable delete wins.
      const snapshot = entry
        ? mergedOf(entry)
        : await fetchSnapshot(id, requestAuth, () => authAccountMatches(requestAuth, get().auth));
      if (creds) await cloudDelete(creds, id);
      if (!authAccountMatches(requestAuth, get().auth)) return;
      if (creds) void clearCloudComposerDraft(creds, id).catch(() => {});
      if (meta?.project && snapshot && (!meta.remoteHost || entry?.remote)) {
        void releaseSnapshotCheckpoints(meta.project, snapshot, entry?.remote ?? null).catch(() => {});
      }
      const cancelsOpening = get().opening?.id === id;
      const clearsVisibleTarget =
        cancelsOpening
        || get().session?.id === id
        || get().unavailableConversation?.id === id;
      if (cancelsOpening) nextSessionEpoch();
      if (
        clearsVisibleTarget
        && get().unavailableCleanupId !== id
      ) {
        useSpecialistStore.getState().close();
      }
      closeLiveSession(get().bridge, id);
      snapshotCache.delete(id);
      removeComposerDraft(draftOwner, id);
      set((state) => ({
        conversations: state.conversations.filter((conversation) => conversation.id !== id),
        runningIds: state.runningIds.filter((runningId) => runningId !== id),
        unseenWorkIds: state.unseenWorkIds.filter((unseenId) => unseenId !== id),
        selectedConversationIds: new Set(
          [...state.selectedConversationIds].filter((selectedId) => selectedId !== id),
        ),
        ...clearActiveConversation(state, id),
        ...settleMutation(state, id, mutationId, true),
      }));
    } catch (error) {
      if (!authAccountMatches(requestAuth, get().auth)) return;
      set((state) => ({
        error: `Could not delete this conversation: ${String(error)}`,
        ...settleMutation(state, id, mutationId, false),
      }));
    }
    await nextMutationPaint();
  }
  clearMutationSummary(set, get, mutationId);
}

async function restoreConversation(
  set: SessionSet,
  get: SessionGet,
  conversationId: string,
): Promise<void> {
  const targets = availableTargets(get, [conversationId], "restore");
  if (targets.length === 0) return;
  // Restoring is a navigation intent, but the cloud acknowledgement can be
  // slow. A later click/new-chat action supersedes it and must not be stolen
  // back when this request eventually completes.
  const navigationEpoch = nextSessionEpoch();
  const mutationId = beginMutation(set, "restore", targets);
  const requestAuth = get().auth;
  const creds = cloudCreds(requestAuth);
  try {
    if (creds) await cloudSetArchived(creds, conversationId, false);
    if (!authAccountMatches(requestAuth, get().auth)) return;
    set((state) => ({
      conversations: state.conversations.map((conversation) =>
        conversation.id === conversationId ? { ...conversation, archived: false } : conversation,
      ),
      ...settleMutation(state, conversationId, mutationId, true),
    }));
    // Restoration is only complete from the user's perspective once their
    // conversation opens. The existing open flow supplies the loading state for
    // a cold reattach and is instant for a live session.
    if (!epochStale(navigationEpoch)) void get().openConversation(conversationId);
  } catch (error) {
    if (!authAccountMatches(requestAuth, get().auth)) return;
    set((state) => ({
      error: `Could not restore this conversation: ${String(error)}`,
      ...settleMutation(state, conversationId, mutationId, false),
    }));
  }
  await nextMutationPaint();
  clearMutationSummary(set, get, mutationId);
}

export function createSidebarConversationActions(
  set: SessionSet,
  get: SessionGet,
): SidebarConversationActions {
  return {
    archiveConversation: (id) => archiveConversations(set, get, [id]),
    restoreConversation: (id) => restoreConversation(set, get, id),
    deleteConversation: (id) => deleteConversations(set, get, [id]),
    toggleConversationSelection: (id) =>
      set((state) => {
        if (state.mutatingConversationIds.has(id)) return state;
        const selectedConversationIds = new Set(state.selectedConversationIds);
        if (selectedConversationIds.has(id)) selectedConversationIds.delete(id);
        else selectedConversationIds.add(id);
        return { selectedConversationIds };
      }),
    setConversationSelection: (ids) =>
      set((state) => ({
        selectedConversationIds: new Set(
          [...ids].filter((id) => !state.mutatingConversationIds.has(id)),
        ),
      })),
    archiveSelectedConversations: () => archiveConversations(set, get, [...get().selectedConversationIds]),
    deleteSelectedConversations: () => deleteConversations(set, get, [...get().selectedConversationIds]),
  };
}
