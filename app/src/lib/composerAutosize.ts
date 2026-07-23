import { useCallback, useLayoutEffect, type RefObject } from "react";

export const COMPOSER_MAX_HEIGHT = 200;

type AutosizeTextarea = Pick<HTMLTextAreaElement, "scrollHeight" | "style">;

/** Reset before reading scrollHeight so an old inline height cannot become the
 * next measurement. This is especially important in WebKit after submission. */
export function resizeComposerTextarea(
  textarea: AutosizeTextarea,
  expandForAttachments: boolean,
  maxHeight = COMPOSER_MAX_HEIGHT,
): void {
  if (!expandForAttachments) {
    textarea.style.height = "";
    return;
  }
  textarea.style.height = "0px";
  textarea.style.height = `${Math.min(textarea.scrollHeight, maxHeight)}px`;
}

type WidthObserver = Pick<ResizeObserver, "observe" | "disconnect">;
type WidthObserverFactory = (callback: ResizeObserverCallback) => WidthObserver;
type FrameScheduler = (callback: FrameRequestCallback) => number;
type FrameCanceller = (handle: number) => void;

/** Remeasure when wrapping width changes. Height-only observer notifications
 * are ignored so updating the inline height cannot start an observer loop. */
export function observeTextareaWidth(
  textarea: HTMLTextAreaElement,
  resize: () => void,
  createObserver: WidthObserverFactory = (callback) => new ResizeObserver(callback),
  scheduleFrame: FrameScheduler = requestAnimationFrame,
  cancelFrame: FrameCanceller = cancelAnimationFrame,
): () => void {
  let previousWidth: number | undefined;
  let pendingFrame: number | null = null;
  const observer = createObserver((entries) => {
    const width = entries.find((entry) => entry.target === textarea)?.contentRect.width;
    if (width === undefined || width === previousWidth) return;
    previousWidth = width;
    if (pendingFrame !== null) cancelFrame(pendingFrame);
    // Writing height during ResizeObserver delivery can itself produce a loop
    // warning. Defer the write until the browser begins its next frame.
    pendingFrame = scheduleFrame(() => {
      pendingFrame = null;
      resize();
    });
  });
  observer.observe(textarea);
  return () => {
    observer.disconnect();
    if (pendingFrame !== null) cancelFrame(pendingFrame);
  };
}

/** Keep ordinary text entry to its single `rows={1}` line. Attachment mode may
 * grow with the content and is remeasured when its wrapping width changes. */
export function useComposerAutosize(
  ref: RefObject<HTMLTextAreaElement | null>,
  value: string,
  expandForAttachments: boolean,
): void {
  const resize = useCallback(() => {
    if (ref.current) resizeComposerTextarea(ref.current, expandForAttachments);
  }, [expandForAttachments, ref]);

  useLayoutEffect(resize, [resize, value]);

  useLayoutEffect(() => {
    const textarea = ref.current;
    if (!textarea || !expandForAttachments) return;

    // ResizeObserver's initial delivery happens after layout has a real width,
    // correcting an early zero-width measurement without guessing at frames.
    if (typeof ResizeObserver !== "undefined") {
      return observeTextareaWidth(textarea, resize);
    }

    // Browser fallback for older webviews.
    window.addEventListener("resize", resize);
    return () => window.removeEventListener("resize", resize);
  }, [expandForAttachments, ref, resize]);
}
