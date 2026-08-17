import { useEffect, useMemo, useRef, useState } from "react";
import { layout, prepare } from "@chenglou/pretext";

import { SPEC_TEXT_REVEAL } from "../../lib/motion";

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
 * wrapped height. The accessible copy remains complete and stable throughout. */
export function PretextRevealText({ text, reduceMotion }: { text: string; reduceMotion: boolean | null }) {
  const chunks = useMemo(() => textChunks(text), [text]);
  const [visibleCount, setVisibleCount] = useState(reduceMotion ? chunks.length : Math.min(1, chunks.length));
  const [reservedHeight, setReservedHeight] = useState<number | null>(null);
  const textRef = useRef<HTMLSpanElement>(null);

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

  useEffect(() => {
    const element = textRef.current;
    if (!element || !text) return;
    const measure = () => {
      const style = window.getComputedStyle(element);
      const width = element.getBoundingClientRect().width;
      const fontSize = Number.parseFloat(style.fontSize);
      const lineHeight = Number.parseFloat(style.lineHeight) || fontSize * 1.5;
      if (width <= 0 || !Number.isFinite(lineHeight)) return;
      const font = `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
      const prepared = prepare(text, font, {
        letterSpacing: Number.parseFloat(style.letterSpacing) || 0,
      });
      setReservedHeight(layout(prepared, width, lineHeight).height);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [text]);

  const complete = visibleCount >= chunks.length;
  return (
    <span
      ref={textRef}
      data-qa="spec-pretext-reveal"
      data-pretext-complete={complete ? "true" : "false"}
      className="block"
      style={reservedHeight === null ? undefined : { minHeight: reservedHeight }}
    >
      <span className="sr-only">{text}</span>
      <span aria-hidden="true">
        {chunks.slice(0, visibleCount).join("").trimEnd()}
        {!complete && <span className="ml-0.5 inline-block h-[0.9em] w-px bg-current align-[-0.05em] opacity-60" />}
      </span>
    </span>
  );
}
