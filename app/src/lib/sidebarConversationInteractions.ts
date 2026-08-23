/**
 * Small, framework-free interaction rules for the conversation sidebar. Keeping
 * them here makes the visible list order the single source of truth for both
 * pointer and keyboard range selection.
 */

export type SidebarConversationMutationKind = "archive" | "delete" | "restore";

/** What a click or keypress on a conversation row is asking for: open it,
 *  add or remove it from the selection, or extend the selection to it. */
export type ConversationSelectionIntent = "open" | "toggle" | "range";

export interface SidebarConversationMutation {
  /** Lets a delayed completion announcement ignore a newer operation. */
  id: number;
  kind: SidebarConversationMutationKind;
  total: number;
  completed: number;
  failed: number;
  pending: number;
}

/** Return the inclusive range between the anchor and target in rendered order.
 * If filtering removed the anchor, selecting just the target is the least
 * surprising fallback. */
export function conversationRangeIds(
  orderedIds: readonly string[],
  anchorId: string | null,
  targetId: string,
): string[] {
  const targetIndex = orderedIds.indexOf(targetId);
  if (targetIndex < 0) return [];
  const anchorIndex = anchorId ? orderedIds.indexOf(anchorId) : -1;
  if (anchorIndex < 0) return [targetId];
  const start = Math.min(anchorIndex, targetIndex);
  const end = Math.max(anchorIndex, targetIndex);
  return orderedIds.slice(start, end + 1);
}

/** The next visible row for Shift+Arrow range selection. No wrapping: pressing
 * past either end should leave a user exactly where they are. */
export function adjacentConversationId(
  orderedIds: readonly string[],
  currentId: string,
  direction: -1 | 1,
): string | null {
  const current = orderedIds.indexOf(currentId);
  const next = current + direction;
  return current < 0 || next < 0 || next >= orderedIds.length ? null : orderedIds[next];
}

/** Human-facing progress copy shared by the visible toolbar and its live
 * region. It deliberately describes outcome, not an ambiguous spinner. */
export function conversationMutationStatusLabel(mutation: SidebarConversationMutation): string {
  const noun = mutation.total === 1 ? "conversation" : "conversations";
  const active = {
    archive: "Archiving",
    delete: "Deleting",
    restore: "Restoring",
  }[mutation.kind];
  const past = {
    archive: "Archived",
    delete: "Deleted",
    restore: "Restored",
  }[mutation.kind];

  if (mutation.pending > 0) {
    const settled = mutation.completed + mutation.failed;
    return settled === 0
      ? `${active} ${mutation.total} ${noun}…`
      : `${active} ${settled} of ${mutation.total} ${noun}…`;
  }
  if (mutation.failed > 0) {
    return `${past} ${mutation.completed} of ${mutation.total} ${noun}. ${mutation.failed} failed.`;
  }
  return `${past} ${mutation.total} ${noun}.`;
}
