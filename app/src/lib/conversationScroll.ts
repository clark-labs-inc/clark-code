export interface ConversationScrollState {
  scrollTop: number;
  atBottom: boolean;
}

export const CONVERSATION_BOTTOM_THRESHOLD = 96;

export function isConversationAtBottom(
  scrollHeight: number,
  scrollTop: number,
  clientHeight: number,
): boolean {
  return scrollHeight - scrollTop - clientHeight < CONVERSATION_BOTTOM_THRESHOLD;
}

/** Running and previously pinned conversations always reopen at the latest
 * output. Idle conversations preserve a deliberate scrollback position. */
export function conversationScrollTarget(
  remembered: ConversationScrollState | undefined,
  busy: boolean,
  scrollHeight: number,
): number {
  return busy || !remembered || remembered.atBottom ? scrollHeight : remembered.scrollTop;
}
