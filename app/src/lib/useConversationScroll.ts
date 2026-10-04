import { useCallback, useEffect, useLayoutEffect, useRef, useState, type MouseEvent } from "react";
import type { Snapshot } from "../core-bridge/types";
import { currentActivity } from "./activity";
import { useSessionStore } from "../store/sessionStore";
import {
  conversationScrollTarget,
  isConversationAtBottom,
  isConversationScrollUp,
  shouldFollowConversation,
  type ConversationScrollState,
} from "./conversationScroll";

/** Conversation is intentionally kept mounted while live sessions switch, so
 * the scroll element is shared. Keep its viewport state keyed by conversation
 * instead of leaking one chat's pinned/scrollback state into the next. Module
 * scope also preserves it across the loading screen used for cold reopens. */
const scrollByConversation = new Map<string, ConversationScrollState>();

/** Owns the transcript viewport independently from per-token rendering. */
export function useConversationScroll(
  sessionId: string | undefined,
  reduce: boolean | null,
  timeline: Snapshot["timeline"],
  toolCalls: Snapshot["tool_calls"],
) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const lastScrollTop = useRef(0);
  const scrollFrameRef = useRef<number | null>(null);
  const pinnedScrollActive = useRef(false);
  const scrollingToBottom = useRef(false);
  const upwardWheel = useRef(false);
  // Pin to the bottom only when the user is already there — never yank them up
  // while they're reading scrollback. One rAF coalesces content resizes
  // into a single scroll update.
  const stuck = useRef(true);
  const [atBottom, setAtBottom] = useState(true);
  const cancelPinnedScroll = useCallback(() => {
    if (scrollFrameRef.current !== null) {
      cancelAnimationFrame(scrollFrameRef.current);
      scrollFrameRef.current = null;
    }
    pinnedScrollActive.current = false;
  }, []);
  const schedulePinnedScroll = useCallback(() => {
    if (!sessionId || !stuck.current || scrollFrameRef.current !== null) return;
    pinnedScrollActive.current = true;

    const scroll = () => {
      scrollFrameRef.current = null;
      if (!stuck.current) {
        pinnedScrollActive.current = false;
        return;
      }
      const el = scrollRef.current;
      if (!el) {
        pinnedScrollActive.current = false;
        return;
      }

      const target = Math.max(0, el.scrollHeight - el.clientHeight);
      if (Math.abs(el.scrollTop - target) < 0.5) {
        pinnedScrollActive.current = false;
        return;
      }
      // One frame is enough to coalesce layout changes. Chasing the target with
      // recursive easing keeps WebKit's scrolling/compositing tree active for
      // many frames after every streamed update and makes input feel sticky.
      el.scrollTop = target;
      lastScrollTop.current = el.scrollTop;
      scrollByConversation.set(sessionId, { scrollTop: el.scrollTop, atBottom: true });
      pinnedScrollActive.current = false;
    };

    if (typeof requestAnimationFrame === "undefined") {
      scroll();
      return;
    }
    scrollFrameRef.current = requestAnimationFrame(scroll);
  }, [sessionId]);
  useEffect(() => () => cancelPinnedScroll(), [cancelPinnedScroll]);
  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const previousScrollTop = lastScrollTop.current;
    const nearBottom = isConversationAtBottom(el.scrollHeight, el.scrollTop, el.clientHeight);
    const movedUp = isConversationScrollUp(previousScrollTop, el.scrollTop);
    const userScrolledUp = upwardWheel.current && movedUp;
    upwardWheel.current = false;
    const following = shouldFollowConversation(
      previousScrollTop,
      el.scrollTop,
      nearBottom,
      scrollingToBottom.current || pinnedScrollActive.current,
      userScrolledUp,
    );
    lastScrollTop.current = el.scrollTop;
    if (userScrolledUp) cancelPinnedScroll();
    if (movedUp || nearBottom) scrollingToBottom.current = false;
    stuck.current = following;
    if (sessionId) {
      scrollByConversation.set(sessionId, { scrollTop: el.scrollTop, atBottom: following });
    }
    if (following !== atBottom) setAtBottom(following);
  };
  const noteUpwardWheel = (deltaY: number) => {
    // Record intent; wait for an actual scroll event before changing state. An
    // endpoint bounce can emit a negative wheel delta without moving the
    // transcript, and should not summon a stale "Jump to latest" button.
    if (deltaY < 0) upwardWheel.current = true;
  };
  const scrollToBottom = () => {
    const el = scrollRef.current;
    if (el) {
      cancelPinnedScroll();
      scrollingToBottom.current = true;
      upwardWheel.current = false;
      stuck.current = true;
      setAtBottom(true);
      el.scrollTo({ top: el.scrollHeight, behavior: reduce ? "auto" : "smooth" });
      if (sessionId) {
        scrollByConversation.set(sessionId, { scrollTop: el.scrollHeight, atBottom: true });
      }
    }
  };

  const onDisclosureClick = (event: MouseEvent<HTMLDivElement>) => {
    const target = event.target;
    if (!(target instanceof Element) || !target.closest("button[aria-expanded]")) return;
    // Opening details is an explicit request to read this row. Its resize must
    // not be mistaken for new output and pull the clicked header out of view.
    cancelPinnedScroll();
    scrollingToBottom.current = false;
    stuck.current = false;
    setAtBottom(false);
    const el = scrollRef.current;
    if (sessionId && el) {
      scrollByConversation.set(sessionId, { scrollTop: el.scrollTop, atBottom: false });
    }
  };

  // Restore after React has committed the target transcript but before paint,
  // avoiding a frame at the previous conversation's unrelated scrollTop.
  useLayoutEffect(() => {
    cancelPinnedScroll();
    const el = scrollRef.current;
    if (!el || !sessionId) return;
    const remembered = scrollByConversation.get(sessionId);
    const busy = currentActivity(useSessionStore.getState().snapshot).busy;
    el.scrollTop = conversationScrollTarget(remembered, busy, el.scrollHeight);
    lastScrollTop.current = el.scrollTop;
    scrollingToBottom.current = false;
    const bottom = isConversationAtBottom(el.scrollHeight, el.scrollTop, el.clientHeight);
    stuck.current = bottom;
    setAtBottom(bottom);
    scrollByConversation.set(sessionId, { scrollTop: el.scrollTop, atBottom: bottom });
  }, [cancelPinnedScroll, sessionId]);

  useEffect(() => {
    schedulePinnedScroll();
  }, [schedulePinnedScroll, sessionId, timeline, toolCalls]);

  // Timeline rows, images, and animated pending/permission banners can change
  // height after their snapshot render. Follow the actual content box while
  // pinned so "latest" remains truly visible rather than a few pixels below
  // the viewport after an enter animation settles.
  useEffect(() => {
    const el = scrollRef.current;
    const content = contentRef.current;
    if (!el || !content || !sessionId || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      if (stuck.current) schedulePinnedScroll();
      else setAtBottom(isConversationAtBottom(el.scrollHeight, el.scrollTop, el.clientHeight));
    });
    observer.observe(content);
    return () => observer.disconnect();
  }, [schedulePinnedScroll, sessionId]);

  return { scrollRef, contentRef, onScroll, noteUpwardWheel, onDisclosureClick, atBottom, scrollToBottom };
}
