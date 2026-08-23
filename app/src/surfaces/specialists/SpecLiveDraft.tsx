import { memo, useMemo } from "react";
import { PenLine } from "lucide-react";
import { cn } from "../../lib/cn";
import {
  endsInsideCodeFence,
  splitStreamingMarkdown,
  type SpecLiveDraft as Draft,
} from "../../lib/specLiveDraft";
import { MarkdownContent, MARKDOWN_CLASSES } from "../MarkdownContent";
import { SpecStreamingLine } from "./SpecStreamingLine";

/** A list marker is pure syntax: its settled form is an unambiguous bullet, so
 *  showing `- ` mid-stream is a seam against the bullets directly above it.
 *  Heading hashes are left alone deliberately — `## ` tells the reader a section
 *  is arriving, which is information, not noise. */
function liveLineParts(line: string): { marker?: string; text: string } {
  const list = /^(\s*)(?:[-*+]|\d+[.)])\s+(.*)$/.exec(line);
  return list ? { marker: "\u2022", text: list[2] } : { text: line };
}

/** The document as the model types it, before any file exists.
 *
 *  Settled lines render as real Markdown — headings, lists and tables become
 *  themselves the moment their line closes. The one line still being written is
 *  handed to Pretext instead, which knows its wrapped geometry before it paints,
 *  so the caret advances without the settled prose above it moving. */
function SpecLiveDraftImpl({ draft, className }: { draft: Draft; className?: string }) {
  const { settled, live } = useMemo(() => splitStreamingMarkdown(draft.text), [draft.text]);
  const liveIsCode = useMemo(() => endsInsideCodeFence(settled), [settled]);
  const revision = draft.kind === "revision";

  return (
    <section
      data-qa="spec-live-draft"
      data-draft-kind={draft.kind}
      aria-label={revision ? "Incoming revision" : "The specification being written"}
      className={cn("mx-auto max-w-[44rem]", className)}
    >
      {revision && (
        <div className="mb-3 flex items-center gap-2 border-b border-border-subtle pb-2 font-mono text-xs text-accent">
          <PenLine aria-hidden className="size-3.5" />
          <span>Writing a revision</span>
          {draft.path && <span className="truncate text-ink-faint">{draft.path}</span>}
        </div>
      )}

      {/* The visible document is the accessible one: a screen reader gets the
          settled Markdown, and the panel announces only that writing is under
          way — never one message per streamed token. */}
      <p className="sr-only" aria-live="polite" aria-atomic="true">
        {revision ? "Writing a revision." : "Writing the specification."}
      </p>

      <div
        className={cn(
          MARKDOWN_CLASSES,
          "pb-4 text-sm leading-7",
          "[&_h1]:font-serif [&_h1]:text-4xl [&_h1]:font-semibold [&_h1]:tracking-[-0.035em]",
          "[&_h2]:mt-8 [&_h2]:border-t [&_h2]:border-border-subtle [&_h2]:pt-6 [&_h2]:font-serif [&_h2]:text-xl",
          revision && "font-mono text-xs leading-6",
        )}
      >
        {settled && <MarkdownContent repairIncomplete>{settled}</MarkdownContent>}
        {live && <LiveLine line={live} code={liveIsCode} revision={revision} />}
      </div>
    </section>
  );
}

function LiveLine({ line, code, revision }: { line: string; code: boolean; revision: boolean }) {
  // Inside an open fence the line is code: a leading `- ` is diff or shell
  // syntax, so no bullet transform, and the type must already be monospace —
  // becoming code only when the fence closes would make the line visibly snap.
  if (code) {
    return <SpecStreamingLine text={line} className="font-mono text-xs leading-6 text-ink-secondary" />;
  }
  // A revision panel sets its settled text in compact mono; the line being
  // typed has to match, or every keystroke lands a size larger than the text
  // it is about to join.
  const lineClass = revision ? "font-mono text-xs leading-6" : "text-sm leading-7";
  const { marker, text } = liveLineParts(line);
  if (!marker) return <SpecStreamingLine text={text} className={lineClass} />;
  return (
    <div className={cn("grid grid-cols-[1.25rem_minmax(0,1fr)]", lineClass)}>
      <span aria-hidden="true" className="text-ink-faint">{marker}</span>
      <SpecStreamingLine text={text} className={lineClass} />
    </div>
  );
}

/** Snapshots re-clone every token, so re-render only on real text growth. */
export const SpecLiveDraft = memo(
  SpecLiveDraftImpl,
  (a, b) =>
    a.className === b.className
    && a.draft.callId === b.draft.callId
    && a.draft.kind === b.draft.kind
    && a.draft.settled === b.draft.settled
    && a.draft.text === b.draft.text,
);
