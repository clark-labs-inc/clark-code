import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";

import type { SpecDiffRow, SpecDocumentDiff as SpecDocumentDiffValue } from "../../lib/specDiff";
import {
  accessibleMotion,
  DUR,
  EXPAND,
  FADE,
  RISE_SMALL,
  staggeredTransition,
} from "../../lib/motion";
import { cn } from "../../lib/cn";

export interface LiveSpecDocumentDiff extends SpecDocumentDiffValue {
  revision: number;
}

type LineStyle = "blank" | "rule" | "h1" | "h2" | "h3" | "list" | "table" | "body";

function linePresentation(raw: string): { style: LineStyle; text: string; marker?: string } {
  const line = raw.trim();
  if (!line) return { style: "blank", text: "" };
  if (/^(?:-{3,}|\*{3,}|_{3,})$/.test(line)) return { style: "rule", text: "" };

  const heading = /^(#{1,3})\s+(.+?)\s*#*$/.exec(line);
  if (heading) {
    return {
      style: `h${heading[1].length}` as "h1" | "h2" | "h3",
      text: heading[2],
    };
  }

  const list = /^(?:[-*+]\s+|(\d+[.)])\s+)(.*)$/.exec(line);
  if (list) return { style: "list", marker: list[1] ?? "•", text: list[2] };

  if (/^\|.*\|$/.test(line)) {
    if (/^\|?(?:\s*:?-+:?\s*\|)+\s*$/.test(line)) return { style: "rule", text: "" };
    return {
      style: "table",
      text: line.replace(/^\|\s?/, "").replace(/\s?\|$/, "").replace(/\s*\|\s*/g, "  ·  "),
    };
  }

  return {
    style: "body",
    text: line
      .replace(/^>\s?/, "")
      .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
      .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
      .replace(/\*\*|__|~~|`/g, ""),
  };
}

function lineClasses(style: LineStyle, pairedReplacement: boolean): string {
  switch (style) {
    case "blank": return "h-3";
    case "rule": return "my-3 h-px bg-border-subtle";
    case "h1": return cn(
      "py-2 font-serif text-4xl font-semibold leading-tight tracking-[-0.035em]",
      pairedReplacement ? "mt-0" : "mt-1",
    );
    case "h2": return cn(
      "border-t border-border-subtle pb-1 font-serif text-xl font-semibold leading-7",
      pairedReplacement ? "mt-1 pt-2" : "mt-8 pt-6",
    );
    case "h3": return cn(
      "font-serif text-lg font-semibold leading-7",
      pairedReplacement ? "mt-1" : "mt-6",
    );
    case "list": return "py-0.5 text-sm leading-7";
    case "table": return "py-1 font-mono text-xs leading-5";
    default: return "py-1 text-sm leading-7";
  }
}

function DiffLine({
  row,
  previous,
  changeIndex,
  reduceMotion,
}: {
  row: SpecDiffRow;
  previous?: SpecDiffRow;
  changeIndex: number;
  reduceMotion: boolean | null;
}) {
  const presentation = linePresentation(row.text);
  const previousPresentation = previous ? linePresentation(previous.text) : null;
  const pairedReplacement = row.kind === "add"
    && previous?.kind === "remove"
    && presentation.style === previousPresentation?.style;
  const enter = row.kind === "add" ? RISE_SMALL : row.kind === "remove" ? FADE : null;
  const exit = accessibleMotion(EXPAND, reduceMotion).exit;

  return (
    <m.div
      layout="position"
      data-diff-kind={row.kind}
      {...(enter ? accessibleMotion(enter, reduceMotion) : { initial: false })}
      exit={exit}
      transition={row.kind === "equal"
        ? { duration: reduceMotion ? DUR.fast : 0.2 }
        : staggeredTransition(reduceMotion, changeIndex, 0.035)}
      className={cn(
        "grid min-w-0 grid-cols-[1.5rem_minmax(0,1fr)] rounded-md",
        row.kind === "add" && presentation.style !== "blank" && "bg-success/10 text-success",
        row.kind === "remove" && presentation.style !== "blank" && "bg-danger/10 text-danger",
      )}
    >
      <span
        aria-hidden="true"
        className={cn(
          "select-none pt-1 text-center font-mono text-xs font-semibold leading-7",
          (row.kind === "equal" || presentation.style === "blank") && "text-transparent",
          presentation.style === "blank" && "h-3 p-0 leading-none",
        )}
      >
        {row.kind === "add" ? "+" : row.kind === "remove" ? "−" : "·"}
      </span>
      <div
        className={cn(
          "min-w-0 pr-2",
          lineClasses(presentation.style, pairedReplacement),
          row.kind === "remove" && presentation.style !== "rule" && "line-through opacity-80",
          row.kind === "equal" && "text-ink",
        )}
      >
        {presentation.style === "list" && (
          <span aria-hidden="true" className="mr-2 text-ink-faint">{presentation.marker}</span>
        )}
        {presentation.text}
      </div>
    </m.div>
  );
}

/** The full document becomes the change surface for one saved revision. Old
 * rows collapse away, additions settle in place, then the parent swaps this
 * transient view for the canonical rendered Markdown. */
export function SpecDocumentDiff({ diff }: { diff: LiveSpecDocumentDiff }) {
  const reduceMotion = useReducedMotion();
  const [showRemoved, setShowRemoved] = useState(true);
  const changeIndexes = useMemo(() => {
    let current = 0;
    return diff.rows.map((row) => {
      if (row.kind === "equal") return current;
      const index = current;
      current += 1;
      return index;
    });
  }, [diff.rows]);

  useEffect(() => {
    setShowRemoved(true);
    const timer = window.setTimeout(() => setShowRemoved(false), reduceMotion ? 700 : 1_050);
    return () => window.clearTimeout(timer);
  }, [diff.revision, reduceMotion]);

  return (
    <div
      data-qa="spec-document-diff"
      role="status"
      aria-live="polite"
      aria-label={`Applying document revision with ${diff.added} additions and ${diff.removed} removals`}
      className="pb-16"
    >
      <div className="mb-3 flex items-center justify-between gap-3 border-b border-border-subtle pb-2 font-mono text-xs">
        <span className="text-accent">Applying revision</span>
        <span className="flex shrink-0 gap-2 tabular-nums">
          {diff.added > 0 && <span className="text-success">+{diff.added}</span>}
          {diff.removed > 0 && <span className="text-danger">−{diff.removed}</span>}
        </span>
      </div>
      <AnimatePresence initial={!reduceMotion}>
        {diff.rows.map((row, index) => {
          if (row.kind === "remove" && !showRemoved) return null;
          return (
            <DiffLine
              key={`${row.kind}:${row.previousLine ?? "new"}:${row.nextLine ?? "old"}:${row.text}`}
              row={row}
              previous={diff.rows[index - 1]}
              changeIndex={changeIndexes[index]}
              reduceMotion={reduceMotion}
            />
          );
        })}
      </AnimatePresence>
    </div>
  );
}
