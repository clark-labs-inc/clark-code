import { useEffect, useMemo, useRef, useState } from "react";
import { layoutWithLines, prepareWithSegments } from "@chenglou/pretext";
import type { LayoutLine } from "@chenglou/pretext";

/** Canvas font string for an element's resolved style.
 *
 *  Pretext measures on a canvas, so the family stack must resolve to the same
 *  concrete face the DOM picked. Every stack in `index.css` is concrete-first
 *  ("DM Sans Variable", "JetBrains Mono", "Newsreader Variable"), with
 *  `system-ui` only deep in the fallback chain — which matters because
 *  `system-ui` is documented as unreliable to measure on macOS. A generic-first
 *  stack would silently degrade accuracy here. */
function canvasFont(style: CSSStyleDeclaration): string {
  return `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
}

interface PretextGeometry {
  /** Wrapped lines of the current text at the measured width. */
  lines: LayoutLine[];
  /** Height the text occupies, already clamped to at least one line — pretext
   *  reports 0 for empty input, where a browser still reserves a line box. */
  height: number;
  lineHeight: number;
  ready: boolean;
}

interface Metrics {
  width: number;
  lineHeight: number;
  font: string;
  letterSpacing: number;
}

/** Off-DOM line geometry for `text` inside `ref`'s content box.
 *
 *  The split matters for cost. `prepareWithSegments` runs `Intl.Segmenter` and a
 *  canvas measurement per segment, and is width-independent — so it is memoized
 *  on the text and font alone. `layoutWithLines` is pure arithmetic over cached
 *  widths, so re-running it per resize tick is free. Measuring both together (as
 *  a single effect keyed on a ResizeObserver would) re-segments the whole string
 *  on every pixel of a drag. */
export function usePretextGeometry(
  ref: React.RefObject<HTMLElement | null>,
  text: string,
): PretextGeometry {
  const [metrics, setMetrics] = useState<Metrics | null>(null);
  // Written by the same effect that owns the observer; compared to skip the
  // state write when a resize tick changes nothing that layout depends on.
  const lastRef = useRef<Metrics | null>(null);

  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    const read = () => {
      const style = window.getComputedStyle(element);
      const width = element.getBoundingClientRect().width;
      const fontSize = Number.parseFloat(style.fontSize);
      const lineHeight = Number.parseFloat(style.lineHeight) || fontSize * 1.5;
      if (!(width > 0) || !Number.isFinite(lineHeight)) return;
      const next: Metrics = {
        width,
        lineHeight,
        font: canvasFont(style),
        letterSpacing: Number.parseFloat(style.letterSpacing) || 0,
      };
      const last = lastRef.current;
      if (
        last
        && last.width === next.width
        && last.lineHeight === next.lineHeight
        && last.font === next.font
        && last.letterSpacing === next.letterSpacing
      ) {
        return;
      }
      lastRef.current = next;
      setMetrics(next);
    };
    read();
    const observer = new ResizeObserver(read);
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref]);

  // Width-independent, so a resize never re-segments.
  const prepared = useMemo(
    () => (metrics && text
      ? prepareWithSegments(text, metrics.font, { letterSpacing: metrics.letterSpacing })
      : null),
    [metrics?.font, metrics?.letterSpacing, text],
  );

  return useMemo(() => {
    if (!prepared || !metrics) {
      return { lines: [], height: 0, lineHeight: metrics?.lineHeight ?? 0, ready: false };
    }
    const result = layoutWithLines(prepared, metrics.width, metrics.lineHeight);
    return {
      lines: result.lines,
      height: Math.max(1, result.lineCount) * metrics.lineHeight,
      lineHeight: metrics.lineHeight,
      ready: true,
    };
  }, [metrics, prepared]);
}
