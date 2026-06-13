import { useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import {
  FileText, FilePen, SquareTerminal, Search, Globe, Trash2, FolderInput,
  Sparkles, Wrench, Check, X, Loader2, ChevronRight,
} from "lucide-react";
import { cn } from "../../lib/cn";
import { lastProgressLine } from "../../lib/activity";
import type { ContentBlock, ToolCall, ToolKind, ToolStatus } from "../../core-bridge/types";

const KIND_ICON: Record<ToolKind, typeof FileText> = {
  read: FileText, edit: FilePen, delete: Trash2, move: FolderInput,
  search: Search, execute: SquareTerminal, think: Sparkles, fetch: Globe, other: Wrench,
};

const KIND_VERB: Record<ToolKind, string> = {
  read: "Read", edit: "Edit", delete: "Delete", move: "Move",
  search: "Search", execute: "Run", think: "Think", fetch: "Fetch", other: "",
};

function blocksText(blocks: ContentBlock[]): string {
  return blocks.map((b) => (b.type === "text" ? b.text : `[${b.type}]`)).join("");
}

function StatusGlyph({ status }: { status: ToolStatus }) {
  if (status === "completed")
    return <Check className="size-3.5 text-success" aria-label="done" />;
  if (status === "failed")
    return <X className="size-3.5 text-danger" aria-label="failed" />;
  if (status === "in_progress")
    return <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite] text-accent" aria-label="in progress" />;
  return <span className="size-1.5 rounded-full bg-ink-faint" aria-label="pending" />;
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

function Detail({ call }: { call: ToolCall }) {
  const raw = blocksText(call.content);
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

/** A single dense, expandable line of agent work (file/browser/terminal/tool). */
export function WorkLine({ call, active }: { call: ToolCall; active: boolean }) {
  const [open, setOpen] = useState(false);
  const reduce = useReducedMotion();
  const Icon = KIND_ICON[call.kind] ?? Wrench;
  const target = call.locations[0]?.path;
  const line = call.locations[0]?.line;
  const hasDetail = call.content.length > 0;
  const progressLine = active ? lastProgressLine(call) : undefined;

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
          "flex w-full items-center gap-2 px-2.5 py-1 text-left text-[0.8125rem] leading-5",
          hasDetail && "cursor-pointer hover:bg-bg-hover/60",
        )}
      >
        <ChevronRight
          className={cn(
            "size-3 shrink-0 text-ink-faint transition-transform",
            !hasDetail && "opacity-0",
            open && "rotate-90",
          )}
        />
        <Icon className="size-3.5 shrink-0 text-ink-muted" />
        {target ? (
          <>
            {KIND_VERB[call.kind] && (
              <span className="shrink-0 font-medium text-ink-muted">{KIND_VERB[call.kind]}</span>
            )}
            <span className="truncate font-mono text-xs text-ink-secondary">
              {target}
              {line ? <span className="text-ink-faint">:{line}</span> : null}
            </span>
          </>
        ) : (
          <span
            className={cn(
              "truncate",
              call.kind === "execute"
                ? "font-mono text-xs text-ink-secondary"
                : "font-medium text-ink",
            )}
          >
            {call.title}
          </span>
        )}
        <span className="ml-auto shrink-0 pl-2">
          <StatusGlyph status={call.status} />
        </span>
      </button>

      {active && !open && progressLine && (
        <div className="truncate pb-1 pl-[1.9rem] pr-2.5 text-xs text-ink-muted">
          {progressLine}
        </div>
      )}

      <AnimatePresence initial={false}>
        {open && hasDetail && (
          <motion.div
            initial={reduce ? false : { height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={reduce ? { opacity: 0 } : { height: 0, opacity: 0 }}
            transition={{ duration: 0.18, ease: [0.4, 0, 0.2, 1] }}
            className="overflow-hidden border-t border-border-subtle bg-bg-secondary/40"
          >
            <div className="max-h-44 overflow-auto">
              <Detail call={call} />
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.div>
  );
}
