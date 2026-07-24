export interface ConversationScrollState {
  scrollTop: number;
  atBottom: boolean;
}

export const CONVERSATION_BOTTOM_THRESHOLD = 96;
const SCROLL_DIRECTION_EPSILON = 0.5;

export function isConversationAtBottom(
  scrollHeight: number,
  scrollTop: number,
  clientHeight: number,
): boolean {
  return scrollHeight - scrollTop - clientHeight < CONVERSATION_BOTTOM_THRESHOLD;
}

/** Upward movement is explicit user intent to read scrollback, even while the
 * viewport is still inside the generous near-bottom threshold. */
export function isConversationScrollUp(
  previousScrollTop: number,
  scrollTop: number,
): boolean {
  return scrollTop < previousScrollTop - SCROLL_DIRECTION_EPSILON;
}

export function shouldFollowConversation(
  previousScrollTop: number,
  scrollTop: number,
  nearBottom: boolean,
  scrollingToBottom = false,
): boolean {
  if (isConversationScrollUp(previousScrollTop, scrollTop)) return false;
  return scrollingToBottom || nearBottom;
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
