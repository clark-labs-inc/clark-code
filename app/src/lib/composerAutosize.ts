import { useCallback, useLayoutEffect, type RefObject } from "react";

export const COMPOSER_MAX_HEIGHT = 200;

type AutosizeTextarea = Pick<HTMLTextAreaElement, "scrollHeight" | "style">;

/** Reset before reading scrollHeight so an old inline height cannot become the
 * next measurement. This is especially important in WebKit after submission. */
export function resizeComposerTextarea(
  textarea: AutosizeTextarea,
  maxHeight = COMPOSER_MAX_HEIGHT,
): void {
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

/** Keep the controlled textarea at its content height, including when its
 * wrapping width changes after mount. */
export function useComposerAutosize(
  ref: RefObject<HTMLTextAreaElement | null>,
  value: string,
): void {
  const resize = useCallback(() => {
    if (ref.current) resizeComposerTextarea(ref.current);
  }, [ref]);

  useLayoutEffect(resize, [resize, value]);

  useLayoutEffect(() => {
    const textarea = ref.current;
    if (!textarea) return;

    // ResizeObserver's initial delivery happens after layout has a real width,
    // correcting an early zero-width measurement without guessing at frames.
    if (typeof ResizeObserver !== "undefined") {
      return observeTextareaWidth(textarea, resize);
    }

    // Browser fallback for older webviews.
    window.addEventListener("resize", resize);
    return () => window.removeEventListener("resize", resize);
  }, [ref, resize]);
}
