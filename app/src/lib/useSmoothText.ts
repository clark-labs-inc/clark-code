import { useEffect, useLayoutEffect, useRef, useState } from "react";

/** How much of the backlog to reveal per frame. Draining ~1/12 per frame at
 *  60fps clears a burst in roughly a quarter second — fast enough to never
 *  fall behind the model, slow enough to read as continuous typing. */
const DRAIN_RATIO = 12;
/** Floor so short backlogs still advance visibly every frame. */
const MIN_CHARS_PER_FRAME = 2;
/** Cap the reveal cadence. At a full 60fps every rAF tick re-slices the string
 *  twice the work of the provider's own token rate for no visible gain. ~30fps
 *  still reads as smooth typing. */
const FRAME_MS = 33;

const graphemeSegmenter =
  typeof Intl !== "undefined" && typeof Intl.Segmenter === "function"
    ? new Intl.Segmenter(undefined, { granularity: "grapheme" })
    : null;

function isHighSurrogate(value: number): boolean {
  return value >= 0xd800 && value <= 0xdbff;
}

function isLowSurrogate(value: number): boolean {
  return value >= 0xdc00 && value <= 0xdfff;
}

function isCombiningOrEmojiModifier(value: number): boolean {
  return (
    (value >= 0x0300 && value <= 0x036f)
    || (value >= 0x1ab0 && value <= 0x1aff)
    || (value >= 0x1dc0 && value <= 0x1dff)
    || (value >= 0x20d0 && value <= 0x20ff)
    || (value >= 0xfe00 && value <= 0xfe2f)
    || (value >= 0x1f3fb && value <= 0x1f3ff)
    || (value >= 0xe0100 && value <= 0xe01ef)
  );
}

/** Fallback for runtimes without `Intl.Segmenter`. It preserves surrogate
 * pairs, combining marks, variation selectors, and common emoji ZWJ sequences.
 * Modern Tauri WebViews take the fully Unicode-correct Segmenter path above. */
function fallbackGraphemeBoundary(text: string, start: number, desired: number): number {
  let end = Math.min(text.length, Math.max(start + 1, desired));
  if (
    end < text.length
    && isHighSurrogate(text.charCodeAt(end - 1))
    && isLowSurrogate(text.charCodeAt(end))
  ) {
    end += 1;
  }

  while (end < text.length) {
    const point = text.codePointAt(end);
    if (point === undefined) break;
    if (isCombiningOrEmojiModifier(point)) {
      end += point > 0xffff ? 2 : 1;
      continue;
    }
    if (point === 0x200d) {
      end += 1;
      const joined = text.codePointAt(end);
      if (joined === undefined) break;
      end += joined > 0xffff ? 2 : 1;
      continue;
    }
    break;
  }
  return end;
}

function graphemeBoundaryAtOrAfter(text: string, start: number, desired: number): number {
  if (desired >= text.length) return text.length;
  if (!graphemeSegmenter) return fallbackGraphemeBoundary(text, start, desired);

  const segments = graphemeSegmenter.segment(text);
  let segment = segments.containing(start);
  if (!segment) return text.length;
  let end = segment.index + segment.segment.length;

  while (end < desired) {
    segment = segments.containing(end);
    if (!segment) return text.length;
    const nextEnd = segment.index + segment.segment.length;
    if (nextEnd <= end) return text.length;
    end = nextEnd;
  }
  return end;
}

/** Return the next safe code-unit offset for a streamed-text reveal. Exported
 * for focused Unicode regression tests; callers should normally use the hook. */
export function nextTextRevealPosition(target: string, shown: number): number {
  const current = Math.min(Math.max(0, shown), target.length);
  const backlog = target.length - current;
  if (backlog <= 0) return current;
  const desired = Math.min(
    target.length,
    current + Math.max(MIN_CHARS_PER_FRAME, Math.ceil(backlog / DRAIN_RATIO)),
  );
  return graphemeBoundaryAtOrAfter(target, current, desired);
}

/** Keep a settled message fully visible when it joins a new stream. Exported
 * for lifecycle regression coverage alongside the reveal-step helper. */
export function synchronizedRevealPosition(
  shown: number,
  targetLength: number,
  active: boolean,
  wasActive: boolean,
): number {
  const length = Math.max(0, targetLength);
  if (!active || !wasActive) return length;
  return Math.min(Math.max(0, shown), length);
}

/**
 * Reveal streamed text at a steady left-to-right pace instead of the chunky
 * jumps the provider's token batches arrive in. While `active`, the returned
 * string is a prefix of `target` that catches up a few characters per frame —
 * proportionally faster the further it lags, so it never drifts behind a fast
 * stream. When `active` is false the full text returns immediately.
 */
export function useSmoothText(target: string, active: boolean): string {
  const [shown, setShown] = useState(() => (active ? 0 : target.length));
  const shownRef = useRef(shown);
  const targetRef = useRef(target);
  const activeRef = useRef(active);
  const wasActiveRef = useRef(active);
  const frameRef = useRef<number | null>(null);
  const lastFrameRef = useRef<number | null>(null);
  const scheduleRef = useRef<() => void>(() => undefined);

  // Keep exactly one rAF alive while there is a real backlog. The old version
  // restarted this effect for every token and kept scheduling frames forever
  // after catch-up, making a quiet transcript still do work at 30–60fps.
  useEffect(() => {
    function schedule(): void {
      if (!activeRef.current || frameRef.current !== null) return;
      frameRef.current = requestAnimationFrame(step);
    }

    function step(now: number): void {
      frameRef.current = null;
      if (!activeRef.current) return;

      const value = targetRef.current;
      const current = Math.min(shownRef.current, value.length);
      if (current !== shownRef.current) {
        shownRef.current = current;
        setShown((previous) => (previous === current ? previous : current));
      }
      if (current >= value.length) return;

      const lastFrame = lastFrameRef.current;
      if (lastFrame !== null && now - lastFrame < FRAME_MS) {
        schedule();
        return;
      }

      lastFrameRef.current = now;
      const next = nextTextRevealPosition(value, current);
      shownRef.current = next;
      setShown((previous) => (previous === next ? previous : next));
      if (next < targetRef.current.length) schedule();
    }

    scheduleRef.current = schedule;
    if (activeRef.current && shownRef.current < targetRef.current.length) schedule();

    return () => {
      if (frameRef.current !== null) cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
      scheduleRef.current = () => undefined;
    };
  }, []);

  // Synchronize visible state before paint when streaming starts/stops or a
  // provider replaces its partial text. Starting from an already-settled turn
  // should never replay the content the user has already read.
  useLayoutEffect(() => {
    activeRef.current = active;
    targetRef.current = target;

    if (!active && frameRef.current !== null) {
      cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
    }

    const next = synchronizedRevealPosition(
      shownRef.current,
      target.length,
      active,
      wasActiveRef.current,
    );
    if (shownRef.current !== next) {
      shownRef.current = next;
      setShown((previous) => (previous === next ? previous : next));
    }
    if (!active) lastFrameRef.current = null;
    wasActiveRef.current = active;
  }, [active, target]);

  // A fresh provider delta wakes the idle scheduler; the callback itself stays
  // mounted so its cadence timestamp survives successive token renders.
  useEffect(() => {
    if (active && shownRef.current < target.length) scheduleRef.current();
  }, [active, target]);

  if (!active) return target;
  return target.slice(0, Math.min(shown, target.length));
}
