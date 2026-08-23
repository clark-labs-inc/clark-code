import { useEffect, useMemo, useRef, useState } from "react";

import { SPEC_TEXT_REVEAL } from "../../lib/motion";
import { usePretextGeometry } from "../../lib/pretextLayout";

function textChunks(text: string): string[] {
  return text.match(/\S+\s*/g) ?? (text ? [text] : []);
}

export function specTextRevealInterval(chunkCount: number): number {
  if (chunkCount <= 1) return 0;
  return Math.max(
    SPEC_TEXT_REVEAL.minIntervalMs,
    Math.min(SPEC_TEXT_REVEAL.maxIntervalMs, Math.floor(SPEC_TEXT_REVEAL.maxDurationMs / chunkCount)),
  );
}

/** Reveals a newly saved line word by word while Pretext reserves its final
 * wrapped height. The accessible copy remains complete and stable throughout.
 *
 * `text` is a settled revision, not a stream: the reveal restarts whenever it
 * changes. Growing text belongs in `SpecStreamingLine`. */
export function PretextRevealText({ text, reduceMotion }: { text: string; reduceMotion: boolean | null }) {
  const chunks = useMemo(() => textChunks(text), [text]);
  const [visibleCount, setVisibleCount] = useState(reduceMotion ? chunks.length : Math.min(1, chunks.length));
  const textRef = useRef<HTMLSpanElement>(null);
  // Reserves the final wrapped height so landing words never reflow the page.
  // The shared hook keeps `prepare` off the resize path — it is width-independent,
  // and re-segmenting the whole string on every observer tick was pure waste.
  const geometry = usePretextGeometry(textRef, text);

  useEffect(() => {
    if (reduceMotion || chunks.length <= 1) {
      setVisibleCount(chunks.length);
      return;
    }
    setVisibleCount(1);
    const timer = window.setInterval(() => {
      setVisibleCount((count) => {
        if (count >= chunks.length) {
          window.clearInterval(timer);
          return count;
        }
        return count + 1;
      });
    }, specTextRevealInterval(chunks.length));
    return () => window.clearInterval(timer);
  }, [chunks, reduceMotion]);

  const complete = visibleCount >= chunks.length;
  return (
    <span
      ref={textRef}
      data-qa="spec-pretext-reveal"
      data-pretext-complete={complete ? "true" : "false"}
      className="block"
      style={geometry.ready ? { minHeight: geometry.height } : undefined}
    >
      <span className="sr-only">{text}</span>
      <span aria-hidden="true">
        {chunks.slice(0, visibleCount).join("").trimEnd()}
        {!complete && <span className="ml-0.5 inline-block h-[0.9em] w-px bg-current align-[-0.05em] opacity-60" />}
      </span>
    </span>
  );
}
