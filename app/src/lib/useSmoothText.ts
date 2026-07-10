import { useEffect, useRef, useState } from "react";

/** How much of the backlog to reveal per frame. Draining ~1/12 per frame at
 *  60fps clears a burst in roughly a quarter second — fast enough to never
 *  fall behind the model, slow enough to read as continuous typing. */
const DRAIN_RATIO = 12;
/** Floor so short backlogs still advance visibly every frame. */
const MIN_CHARS_PER_FRAME = 2;

/**
 * Reveal streamed text at a steady left-to-right pace instead of the chunky
 * jumps the provider's token batches arrive in. While `active`, the returned
 * string is a prefix of `target` that catches up a few characters per frame —
 * proportionally faster the further it lags, so it never drifts behind a fast
 * stream. When `active` is false the full text returns immediately.
 */
export function useSmoothText(target: string, active: boolean): string {
  const [shown, setShown] = useState(() => (active ? 0 : target.length));
  // Streaming just started for this message: begin the reveal from the text
  // we first saw rather than replaying content the user already read.
  const wasActive = useRef(active);
  if (active && !wasActive.current) {
    // eslint-disable-next-line react-hooks/exhaustive-deps -- render-time reset, React-sanctioned pattern
    setShown(target.length);
  }
  wasActive.current = active;

  useEffect(() => {
    if (!active) return;
    let raf = 0;
    const step = () => {
      setShown((s) => {
        const backlog = target.length - s;
        if (backlog <= 0) return s;
        return s + Math.max(MIN_CHARS_PER_FRAME, Math.ceil(backlog / DRAIN_RATIO));
      });
      raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
  }, [target, active]);

  if (!active) return target;
  return target.slice(0, Math.min(shown, target.length));
}
