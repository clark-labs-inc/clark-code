import { useRef } from "react";
import { cn } from "../../lib/cn";
import { usePretextGeometry } from "../../lib/pretextLayout";

/** The line the model is currently typing, laid out off-DOM.
 *
 *  Rendering streaming text as ordinary flowed HTML makes the block's height
 *  jump every time a word wraps, which shoves the settled document above it.
 *  Here Pretext reports the wrapped lines before anything paints, so the block
 *  claims its height in one write and each line is positioned rather than
 *  reflowed — the caret advances, the page holds still.
 *
 *  Accessibility: the settled document above this is the readable copy. This is
 *  a transient in-progress line, so it is `aria-hidden` and the surrounding
 *  panel owns the live region. */
export function SpecStreamingLine({ text, className }: { text: string; className?: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const geometry = usePretextGeometry(ref, text);

  return (
    <div
      ref={ref}
      aria-hidden="true"
      data-qa="spec-streaming-line"
      data-pretext-lines={geometry.ready ? geometry.lines.length : undefined}
      // `transition-none` is load-bearing: the reduced-motion block in index.css
      // sets a universal `transition-duration` without constraining
      // `transition-property`, whose initial value is `all` — so a reserved
      // height would animate instead of applying. A reservation must be instant;
      // MOTION.md keeps layout properties out of decoration either way.
      className={cn("relative w-full transition-none", className)}
      style={geometry.ready ? { height: geometry.height } : undefined}
    >
      {geometry.ready
        ? geometry.lines.map((line, index) => (
          <span
            key={index}
            className="absolute left-0 block whitespace-pre"
            style={{ top: index * geometry.lineHeight, lineHeight: `${geometry.lineHeight}px` }}
          >
            {line.text}
            {index === geometry.lines.length - 1 && <StreamCaret />}
          </span>
        ))
        // Before the first measurement there is no geometry to trust; flow the
        // text normally for that one frame rather than showing nothing.
        : <span className="whitespace-pre-wrap">{text}<StreamCaret /></span>}
    </div>
  );
}

/** Marks where the next word will land. `breathe` degrades under the
 *  application-wide reduced-motion block, leaving a static bar. */
function StreamCaret() {
  return (
    <span className="breathe ml-0.5 inline-block h-[0.9em] w-px bg-accent align-[-0.05em]" />
  );
}
