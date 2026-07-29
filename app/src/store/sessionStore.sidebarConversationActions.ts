import type { SidebarConversationMutationKind } from "../lib/sidebarConversationInteractions";
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
  if (state.session?.id !== id) return {};
  return {
    session: null,
    snapshot: emptySnapshot(),
    error: null,
    attachments: [],
    historyPrefix: null,
    queued: [],
    terminalOpen: false,
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
  const creds = cloudCreds(get().auth);

  for (const id of targets) {
    try {
      if (creds) await cloudSetArchived(creds, id, true);
      closeLiveSession(get().bridge, id);
      set((state) => ({
        conversations: state.conversations.map((conversation) =>
          conversation.id === id ? { ...conversation, archived: true } : conversation,
        ),
        runningIds: state.runningIds.filter((runningId) => runningId !== id),
        selectedConversationIds: new Set(
          [...state.selectedConversationIds].filter((selectedId) => selectedId !== id),
        ),
        ...clearActiveConversation(state, id),
        ...settleMutation(state, id, mutationId, true),
      }));
    } catch (error) {
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
  const creds = cloudCreds(get().auth);

  for (const id of targets) {
    const entry = liveSessions.get(id);
    const meta = get().conversations.find((conversation) => conversation.id === id);
    try {
      // Capture checkpoint ownership before cloudDelete tombstones the snapshot
      // pipeline. The release remains best-effort, just as it was before, but
      // never holds the visible deletion hostage after the durable delete wins.
      const snapshot = entry ? mergedOf(entry) : await fetchSnapshot(id, get().auth);
      if (creds) await cloudDelete(creds, id);
      if (meta?.project && snapshot && (!meta.remoteHost || entry?.remote)) {
        void releaseSnapshotCheckpoints(meta.project, snapshot, entry?.remote ?? null).catch(() => {});
      }
      closeLiveSession(get().bridge, id);
      snapshotCache.delete(id);
      set((state) => ({
        conversations: state.conversations.filter((conversation) => conversation.id !== id),
        runningIds: state.runningIds.filter((runningId) => runningId !== id),
        selectedConversationIds: new Set(
          [...state.selectedConversationIds].filter((selectedId) => selectedId !== id),
        ),
        ...clearActiveConversation(state, id),
        ...settleMutation(state, id, mutationId, true),
      }));
    } catch (error) {
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
  const mutationId = beginMutation(set, "restore", targets);
  const creds = cloudCreds(get().auth);
  try {
    if (creds) await cloudSetArchived(creds, conversationId, false);
    set((state) => ({
      conversations: state.conversations.map((conversation) =>
        conversation.id === conversationId ? { ...conversation, archived: false } : conversation,
      ),
      ...settleMutation(state, conversationId, mutationId, true),
    }));
    // Restoration is only complete from the user's perspective once their
    // conversation opens. The existing open flow supplies the loading state for
    // a cold reattach and is instant for a live session.
    void get().openConversation(conversationId);
  } catch (error) {
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
