import { memo, useEffect, useRef, useState } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import {
  FileText, X, Loader2, ChevronRight, ExternalLink, FolderOpen,
} from "lucide-react";
import { cn } from "../../lib/cn";
import { lastProgressLine } from "../../lib/activity";
import { callDiffStat, langFromPath, parseDiff, type DiffStat } from "../../lib/diff";
import { highlightCacheKey, highlightLines } from "../../lib/highlight";
import { ansiToHtml } from "../../lib/ansi";
import { contentText, imageBlocks, imageSource, sameContentBlocks } from "../../lib/contentBlocks";
import { openProjectPath } from "../../lib/openPath";
import {
  DUR,
  EASE,
  REDUCED_EXIT,
  RISE_SMALL,
  accessibleMotion,
} from "../../lib/motion";
import { useSessionStore } from "../../store/sessionStore";
import { ResearchWork } from "./ResearchWork";
import type { ContentBlock, ToolCall, ToolKind, ToolStatus } from "../../core-bridge/types";

const KIND_VERB: Record<ToolKind, string> = {
  read: "Read", edit: "Edit", delete: "Delete", move: "Move",
  search: "Search", execute: "Ran", think: "Think", fetch: "Fetch",
  research: "Researched", view_image: "View", generate_image: "Generate", other: "",
};

function kindVerb(call: ToolCall): string {
  if (call.kind === "execute" && call.status === "in_progress") return "Running";
  return KIND_VERB[call.kind];
}

function blocksText(blocks: ContentBlock[]): string {
  return contentText(blocks);
}

// Completion is implied (no trailing check). Only surface the
// states that need attention — in-progress and failed.
function StatusGlyph({ status }: { status: ToolStatus }) {
  if (status === "failed") return <X className="size-3.5 text-danger" aria-label="failed" />;
  if (status === "cancelled") return <X className="size-3.5 text-ink-faint" aria-label="cancelled" />;
  if (status === "in_progress")
    return (
      <Loader2
        className="size-3.5 animate-[spin_1s_linear_infinite] text-ink-muted"
        aria-label="in progress"
      />
    );
  return null;
}

/** Git-style added/removed counts, e.g. `+42 −3`. */
function DiffStatBadge({ stat }: { stat: DiffStat }) {
  return (
    <span className="shrink-0 font-mono text-xs tabular-nums">
      {stat.adds > 0 && <span className="text-success">+{stat.adds}</span>}
      {stat.adds > 0 && stat.dels > 0 && <span className="text-ink-faint"> </span>}
      {stat.dels > 0 && <span className="text-danger">−{stat.dels}</span>}
    </span>
  );
}

/** Quiet period before tokenizing a diff that is still arriving. Short enough
 *  to feel immediate once a tool call settles, long enough that a streaming
 *  diff tokenizes once instead of once per token. */
const DIFF_HIGHLIGHT_QUIET_MS = 120;

export function DiffBody({ text }: { text: string }) {
  const parsed = parseDiff(text);
  // Not a structured diff — render the old plain-monospace view.
  if (!parsed) {
    const lines = text.split("\n");
    return (
      <pre className="overflow-x-auto px-3 py-2 font-mono text-xs leading-[1.5]">
        {lines.map((line, i) => (
          <div key={i}>{line || " "}</div>
        ))}
      </pre>
    );
  }

  const lang = langFromPath(parsed.path);
  // Collect the in-hunk code lines (prefixes stripped) as one block to highlight
  // in the file's language, so a diff of a Rust file shows colored Rust — not
  // just flat red/green text. lineCodeIdx maps each parsed line → code-block row.
  const codeLines: string[] = [];
  const lineCodeIdx: number[] = [];
  for (const l of parsed.lines) {
    if (l.kind === "context" || l.kind === "add" || l.kind === "del") {
      lineCodeIdx.push(codeLines.length);
      codeLines.push(l.text);
    } else {
      lineCodeIdx.push(-1);
    }
  }
  const [hl, setHl] = useState<string[] | null>(null);
  // What the current rows were rendered from. Keyed on language and source
  // together (source alone would skip a re-highlight when the same text is
  // shown for a different file type), and carrying the source so the effect can
  // tell a diff that grew from one that was replaced.
  const highlightedFor = useRef<{ key: string; lang: string; text: string } | null>(null);
  useEffect(() => {
    const previous = highlightedFor.current;
    if (!lang) {
      // A reused instance whose new content has no language must not keep
      // showing rows tokenized from the previous diff.
      if (previous !== null) {
        highlightedFor.current = null;
        setHl(null);
      }
      return;
    }
    const key = highlightCacheKey(lang, text);
    if (previous?.key === key) return;
    // Rows carry over only while this is the same diff growing (streamed text
    // gains a suffix). A language change or rewritten content means the rows
    // describe some other diff — misaligned colors are worse than plain text.
    if (previous !== null && !(previous.lang === lang && text.startsWith(previous.text))) {
      highlightedFor.current = null;
      setHl(null);
    }
    let alive = true;
    // A diff that is still streaming re-enters this effect on every token, and
    // tokenizing costs more than a frame, so wait for the text to stop changing
    // instead of starting a pass that the next token invalidates. Existing rows
    // stay on screen meanwhile rather than flashing back to plain text.
    const timer = window.setTimeout(() => {
      highlightLines(codeLines.join("\n"), lang).then((rows) => {
        if (!alive || !rows) return;
        highlightedFor.current = { key, lang, text };
        setHl(rows);
      });
    }, DIFF_HIGHLIGHT_QUIET_MS);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
    // codeLines/lang derive from `text`; re-highlight when it changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [text, lang]);

  return (
    <div className="diff-body font-mono text-xs leading-[1.55]">
      <div className="flex items-center gap-2 border-b border-border-subtle px-3 py-1.5">
        <FileText className="size-3.5 shrink-0 text-ink-faint" />
        <span className="min-w-0 flex-1 truncate text-ink-secondary">{parsed.path}</span>
        <span className="shrink-0 tabular-nums text-success">{parsed.stats.adds > 0 && `+${parsed.stats.adds}`}</span>
        <span className="shrink-0 tabular-nums text-danger">{parsed.stats.dels > 0 && `−${parsed.stats.dels}`}</span>
      </div>
      <div className="overflow-x-auto py-1">
        {parsed.lines.map((line, i) => {
          if (line.kind === "meta") {
            if (/^(index |similarity |rename |new file|deleted file|old mode|new mode|copy (from|to))/.test(line.text)) {
              return null;
            }
            return <div key={i} className="diff-meta px-3 text-ink-faint">{line.text}</div>;
          }
          if (line.kind === "hunk") {
            return <div key={i} className="diff-hunk px-3 text-info">{line.text}</div>;
          }
          if (line.kind === "plain") {
            return <div key={i} className="px-3 text-ink-faint">{line.text || " "}</div>;
          }
          const add = line.kind === "add";
          const del = line.kind === "del";
          const oldNo = line.kind === "del" ? line.oldNo : line.kind === "context" ? line.oldNo : null;
          const newNo = line.kind === "add" ? line.newNo : line.kind === "context" ? line.newNo : null;
          const inner = hl && lineCodeIdx[i] >= 0 ? hl[lineCodeIdx[i]] : null;
          return (
            <div key={i} className={cn("diff-row", add && "diff-add", del && "diff-del", !add && !del && "diff-ctx")}>
              <span className="diff-gutter-old">{oldNo ?? ""}</span>
              <span className="diff-gutter-new">{newNo ?? ""}</span>
              <span className={cn("diff-sign", add && "text-success", del && "text-danger")}>
                {add ? "+" : del ? "−" : " "}
              </span>
              <span className="diff-code">
                {inner ? (
                  <span className="shiki-inline" dangerouslySetInnerHTML={{ __html: inner }} />
                ) : (
                  line.text || " "
                )}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

// Hide internal plumbing (index/manifest paths, signatures, runtime bookkeeping)
// so the detail shows the actual result, not leaked internals.
const INTERNAL_LINE =
  /^\s*(query|extracted_index|manifest|artifact_path|artifact_bytes|index|storage_id|execution_storage_id|observation_signature|tool_effect|legacy_tool_name|file_action|size_bytes|append|changed_state|terminate|success)\s*[:=]/i;

function cleanOutput(text: string): string {
  return text
    .split("\n")
    .filter((l) => !INTERNAL_LINE.test(l) && !l.includes("/research_outputs/"))
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

/** Keep typed tool-image outputs visual instead of flattening them into `[image]`. */
export function ToolImages({ blocks }: { blocks: ContentBlock[] }) {
  const images = imageBlocks(blocks);
  if (!images.some((image) => imageSource(image) !== null)) return null;
  return (
    <div className="flex flex-wrap gap-2 px-3 py-2">
      {images.map((image, index) => {
        const src = imageSource(image);
        return src ? (
          <img
            key={`${image.mime_type}:${index}`}
            src={src}
            alt="Tool result image"
            loading="lazy"
            decoding="async"
            className="max-h-64 max-w-full rounded-md border border-border-subtle bg-bg-sunken object-contain"
          />
        ) : null;
      })}
    </div>
  );
}

function Detail({ call }: { call: ToolCall }) {
  const raw = blocksText(call.content);
  const hasImages = imageBlocks(call.content).some((image) => imageSource(image) !== null);
  if (call.kind === "edit" && raw.startsWith("diff ")) {
    return (
      <>
        <ToolImages blocks={call.content} />
        <DiffBody text={raw} />
      </>
    );
  }
  const text = cleanOutput(raw);
  if (!text) {
    return hasImages ? <ToolImages blocks={call.content} /> : <p className="px-3 py-2 text-xs text-ink-faint">No output.</p>;
  }
  if (call.kind === "execute") {
    return (
      <>
        <ToolImages blocks={call.content} />
        <div className="px-3 py-2 font-mono text-xs leading-[1.5]">
          <div className="text-success">$ {call.title}</div>
          <div
            className="whitespace-pre-wrap text-ink-secondary ansi-out"
            dangerouslySetInnerHTML={{ __html: ansiToHtml(text) }}
          />
        </div>
      </>
    );
  }
  return (
    <>
      <ToolImages blocks={call.content} />
      <pre className="overflow-x-auto whitespace-pre-wrap px-3 py-2 font-mono text-xs leading-[1.5] text-ink-secondary">
        {text}
      </pre>
    </>
  );
}

/** A header above an expanded file detail with open / reveal affordances. */
function FileActions({ path }: { path: string }) {
  const cwd = useSessionStore((s) => s.activeProjectRoot ?? "");
  const remote = useSessionStore((s) => s.activeRemote !== null);
  return (
    <div className="flex items-center justify-between gap-2 border-b border-border-subtle px-3 py-1.5">
      <span className="min-w-0 flex-1 truncate font-mono text-xs text-ink-muted">{path}</span>
      {!remote && (
        <span className="flex shrink-0 items-center gap-0.5">
          <button
            onClick={() => void openProjectPath(cwd, path, false)}
            title="Open file"
            aria-label="Open file"
            className="grid size-6 place-items-center rounded-md text-ink-faint transition hover:bg-bg-hover hover:text-ink"
          >
            <ExternalLink className="size-3.5" />
          </button>
          <button
            onClick={() => void openProjectPath(cwd, path, true)}
            title="Reveal in file manager"
            aria-label="Reveal in file manager"
            className="grid size-6 place-items-center rounded-md text-ink-faint transition hover:bg-bg-hover hover:text-ink"
          >
            <FolderOpen className="size-3.5" />
          </button>
        </span>
      )}
    </div>
  );
}

/** A single dense, expandable line of agent work (file/browser/terminal/tool). */
function WorkLineImpl({ call, active }: { call: ToolCall; active: boolean }) {
  const [open, setOpen] = useState(false);
  const reduce = useReducedMotion();
  if (call.kind === "research") return <ResearchWork call={call} active={active} />;

  const target = call.locations?.[0]?.path;
  const line = call.locations?.[0]?.line;
  const hasDetail = call.content.length > 0;
  const progressLine = active ? lastProgressLine(call) : undefined;
  const stat = callDiffStat(call);
  const verb = kindVerb(call);

  return (
    <m.div
      id={`tool-call-${call.id}`}
      data-tool-call-id={call.id}
      tabIndex={-1}
      {...accessibleMotion(RISE_SMALL, reduce)}
      className={cn(
        "outline-none transition-colors focus-visible:ring-2 focus-visible:ring-accent",
        active && "bg-bg-hover/40",
      )}
    >
      <button
        type="button"
        onClick={() => hasDetail && setOpen((v) => !v)}
        aria-expanded={open}
        disabled={!hasDetail}
        className={cn(
          "group flex w-full items-center gap-1.5 rounded-md px-1 py-0.5 text-left text-sm leading-5 text-ink-muted",
          hasDetail && "cursor-pointer hover:bg-bg-hover/50 hover:text-ink-secondary",
        )}
      >
        {target ? (
          <span className="min-w-0 flex-1 truncate font-mono text-xs">
            {verb && <span className="text-ink-faint">{verb} </span>}
            <span className="text-ink-muted">{target}</span>
            {line ? <span className="text-ink-faint">:{line}</span> : null}
          </span>
        ) : (
          <span
            className={cn(
              "min-w-0 flex-1 truncate",
              call.kind === "execute" ? "font-mono text-xs" : "",
            )}
          >
            {call.kind === "execute" && <span className="text-ink-faint">{verb} </span>}
            {call.title}
          </span>
        )}
        <span className="flex shrink-0 items-center gap-1.5 pl-2">
          {stat && <DiffStatBadge stat={stat} />}
          {hasDetail && (
            <ChevronRight
              className={cn(
                "size-3 text-ink-faint opacity-0 transition group-hover:opacity-100",
                open && "rotate-90 opacity-100",
              )}
            />
          )}
          <StatusGlyph status={call.status} />
        </span>
      </button>

      {active && !open && progressLine ? (
        <div className="truncate pb-0.5 pl-[1.4rem] pr-2 text-xs text-ink-faint">{progressLine}</div>
      ) : null}

      <AnimatePresence initial={false}>
        {open && hasDetail && (
          <m.div
            initial={reduce ? { opacity: 0 } : { height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={reduce ? REDUCED_EXIT : { height: 0, opacity: 0 }}
            transition={{ duration: DUR.fast, ease: reduce ? EASE.out : EASE.inOut }}
            className="ml-[0.55rem] overflow-hidden border-l border-border-subtle"
          >
            {target && <FileActions path={target} />}
            <div className="max-h-56 overflow-auto">
              <Detail call={call} />
            </div>
          </m.div>
        )}
      </AnimatePresence>
    </m.div>
  );
}

/** Memoized: snapshots are re-cloned every token, so re-render a tool line only
 *  when something it shows actually changed. */
export const WorkLine = memo(
  WorkLineImpl,
  (a, b) =>
    a.active === b.active &&
    a.call.id === b.call.id &&
    a.call.status === b.call.status &&
    a.call.title === b.call.title &&
    a.call.progress?.revision === b.call.progress?.revision &&
    sameContentBlocks(a.call.content, b.call.content) &&
    a.call.locations.length === b.call.locations.length,
);
