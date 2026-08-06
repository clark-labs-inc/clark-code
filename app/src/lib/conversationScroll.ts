export interface ConversationScrollState {
  scrollTop: number;
  atBottom: boolean;
}

const CONVERSATION_BOTTOM_THRESHOLD = 96;
const SCROLL_DIRECTION_EPSILON = 0.5;
const PINNED_SCROLL_EASING = 0.22;
const PINNED_SCROLL_MIN_STEP = 1;
const PINNED_SCROLL_MAX_STEP = 72;

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
  userScrolledUp = false,
): boolean {
  // A scroll container can clamp its offset when surrounding layout changes
  // (for example, when an in-flow control exits). Only a recorded upward user
  // gesture should turn that mechanical offset decrease into scrollback intent.
  if (userScrolledUp && isConversationScrollUp(previousScrollTop, scrollTop)) return false;
  return scrollingToBottom || nearBottom;
}

/** Advance a pinned transcript toward its moving bottom without snapping.
 * The target is sampled again every animation frame, so streaming text and
 * tool rows can keep growing while the viewport catches up. The cap prevents
 * a tall tool card from flinging the transcript; the floor avoids a long,
 * sub-pixel tail at the end of the animation. */
export function nextPinnedScrollTop(current: number, target: number): number {
  const distance = Math.max(0, target - current);
  if (distance <= SCROLL_DIRECTION_EPSILON) return target;
  const step = Math.min(
    PINNED_SCROLL_MAX_STEP,
    Math.max(PINNED_SCROLL_MIN_STEP, distance * PINNED_SCROLL_EASING),
  );
  return Math.min(target, current + step);
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
