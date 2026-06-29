import { memo, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import {
  FileText, FilePen, SquareTerminal, Search, Globe, Trash2, FolderInput,
  Sparkles, Wrench, X, Loader2, ChevronRight, Telescope, ExternalLink, FolderOpen,
} from "lucide-react";
import { cn } from "../../lib/cn";
import { lastProgressLine } from "../../lib/activity";
import { callDiffStat, type DiffStat } from "../../lib/diff";
import { extractSources } from "../../lib/sources";
import { openExternal } from "../../lib/account";
import { openProjectPath } from "../../lib/openPath";
import { useSessionStore } from "../../store/sessionStore";
import { Md, MD_CLASSES } from "../Message";
import type { ContentBlock, ToolCall, ToolKind, ToolStatus } from "../../core-bridge/types";

const KIND_ICON: Record<ToolKind, typeof FileText> = {
  read: FileText, edit: FilePen, delete: Trash2, move: FolderInput,
  search: Search, execute: SquareTerminal, think: Sparkles, fetch: Globe,
  research: Telescope, other: Wrench,
};

const KIND_VERB: Record<ToolKind, string> = {
  read: "Read", edit: "Edit", delete: "Delete", move: "Move",
  search: "Search", execute: "Ran", think: "Think", fetch: "Fetch",
  research: "Researched", other: "",
};

function blocksText(blocks: ContentBlock[]): string {
  return blocks.map((b) => (b.type === "text" ? b.text : `[${b.type}]`)).join("");
}

// Codex restraint: completion is implied (no trailing check). Only surface the
// states that need attention — in-progress and failed.
function StatusGlyph({ status }: { status: ToolStatus }) {
  if (status === "failed") return <X className="size-3.5 text-danger" aria-label="failed" />;
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
    <span className="shrink-0 font-mono text-[0.7rem] tabular-nums">
      {stat.adds > 0 && <span className="text-success">+{stat.adds}</span>}
      {stat.adds > 0 && stat.dels > 0 && <span className="text-ink-faint"> </span>}
      {stat.dels > 0 && <span className="text-danger">−{stat.dels}</span>}
    </span>
  );
}

function DiffBody({ text }: { text: string }) {
  const lines = text.replace(/^diff .*\n/, "").split("\n");
  return (
    <pre className="overflow-x-auto px-3 py-2 font-mono text-xs leading-[1.5]">
      {lines.map((line, i) => {
        const add = line.startsWith("+");
        const del = line.startsWith("-");
        return (
          <div
            key={i}
            className={cn(
              "-mx-1 px-1",
              add && "bg-success/12 text-success",
              del && "bg-danger/12 text-danger",
              !add && !del && "text-ink-muted",
            )}
          >
            {line || " "}
          </div>
        );
      })}
    </pre>
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

/** The query a clark_research call ran, for the work-line label. */
function researchQuery(call: ToolCall): string {
  const q = (call.raw_input as { query?: string } | undefined)?.query;
  return (q || call.title.replace(/^clark_research:\s*/, "")).trim();
}

/** Clark's research findings — rendered as markdown, with the cited sources
 *  pulled out into clickable chips so the agent's web work is legible + trusted. */
function ResearchDetail({ call }: { call: ToolCall }) {
  const findings = blocksText(call.content).trim();
  const sources = extractSources(findings);
  if (!findings) {
    return <p className="px-3 py-2.5 text-xs text-ink-faint">No findings.</p>;
  }
  return (
    <div className="px-3 py-2.5">
      <div className={cn("text-[0.85rem] leading-relaxed", MD_CLASSES)}>
        <Md>{findings}</Md>
      </div>
      {sources.length > 0 && (
        <div className="mt-3 border-t border-border-subtle pt-2.5">
          <div className="mb-1.5 flex items-center gap-1.5 text-[0.7rem] font-medium uppercase tracking-wide text-ink-faint">
            <Globe className="size-3" /> Sources
          </div>
          <div className="flex flex-wrap gap-1.5">
            {sources.map((s, i) => (
              <button
                key={i}
                onClick={() => void openExternal(s.url)}
                title={s.url}
                className="flex items-center gap-1 rounded-md bg-chip px-2 py-0.5 text-xs text-ink-secondary transition hover:bg-bg-hover hover:text-ink"
              >
                {s.label}
                <ExternalLink className="size-2.5 text-ink-faint" />
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function Detail({ call }: { call: ToolCall }) {
  const raw = blocksText(call.content);
  if (call.kind === "research") {
    return <ResearchDetail call={call} />;
  }
  if (call.kind === "edit" && raw.startsWith("diff ")) {
    return <DiffBody text={raw} />;
  }
  const text = cleanOutput(raw);
  if (!text) {
    return <p className="px-3 py-2 text-xs text-ink-faint">No output.</p>;
  }
  if (call.kind === "execute") {
    return (
      <div className="px-3 py-2 font-mono text-xs leading-[1.5]">
        <div className="text-success">$ {call.title}</div>
        <div className="whitespace-pre-wrap text-ink-secondary">{text}</div>
      </div>
    );
  }
  return (
    <pre className="overflow-x-auto whitespace-pre-wrap px-3 py-2 font-mono text-xs leading-[1.5] text-ink-secondary">
      {text}
    </pre>
  );
}

/** A header above an expanded file detail with open / reveal affordances. */
function FileActions({ path }: { path: string }) {
  const cwd = useSessionStore((s) => s.localSettings.cwd);
  return (
    <div className="flex items-center justify-between gap-2 border-b border-border-subtle px-3 py-1.5">
      <span className="min-w-0 flex-1 truncate font-mono text-xs text-ink-muted">{path}</span>
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
    </div>
  );
}

/** A single dense, expandable line of agent work (file/browser/terminal/tool). */
function WorkLineImpl({ call, active }: { call: ToolCall; active: boolean }) {
  const [open, setOpen] = useState(false);
  const reduce = useReducedMotion();
  const Icon = KIND_ICON[call.kind] ?? Wrench;
  const target = call.locations[0]?.path;
  const line = call.locations[0]?.line;
  const hasDetail = call.content.length > 0;
  const progressLine = active ? lastProgressLine(call) : undefined;
  const stat = callDiffStat(call);

  return (
    <motion.div
      initial={reduce ? false : { opacity: 0, y: 4 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2, ease: [0.4, 0, 0.2, 1] }}
      className={cn("transition-colors", active && "bg-bg-hover/40")}
    >
      <button
        type="button"
        onClick={() => hasDetail && setOpen((v) => !v)}
        aria-expanded={open}
        disabled={!hasDetail}
        className={cn(
          "group flex w-full items-center gap-1.5 rounded-md px-1 py-0.5 text-left text-[0.8125rem] leading-5 text-ink-muted",
          hasDetail && "cursor-pointer hover:bg-bg-hover/50 hover:text-ink-secondary",
        )}
      >
        <Icon className="size-3.5 shrink-0 text-ink-faint" />
        {target ? (
          <span className="min-w-0 flex-1 truncate font-mono text-xs">
            {KIND_VERB[call.kind] && <span className="text-ink-faint">{KIND_VERB[call.kind]} </span>}
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
            {call.kind === "research" ? (
              <>
                <span className="text-ink-faint">{active ? "Researching " : "Researched "}</span>
                {researchQuery(call)}
              </>
            ) : (
              <>
                {call.kind === "execute" && <span className="text-ink-faint">Ran </span>}
                {call.title}
              </>
            )}
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

      {active && !open && progressLine && (
        <div className="truncate pb-0.5 pl-[1.4rem] pr-2 text-xs text-ink-faint">{progressLine}</div>
      )}

      <AnimatePresence initial={false}>
        {open && hasDetail && (
          <motion.div
            initial={reduce ? false : { height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={reduce ? { opacity: 0 } : { height: 0, opacity: 0 }}
            transition={{ duration: 0.18, ease: [0.4, 0, 0.2, 1] }}
            className="ml-[0.55rem] overflow-hidden border-l border-border-subtle"
          >
            {target && call.kind !== "research" && <FileActions path={target} />}
            <div className="max-h-56 overflow-auto">
              <Detail call={call} />
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.div>
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
    a.call.content.length === b.call.content.length &&
    a.call.locations.length === b.call.locations.length,
);
