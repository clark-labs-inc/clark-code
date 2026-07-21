import { useEffect, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { ChevronRight, ExternalLink, Globe2, Loader2, X } from "lucide-react";
import { cn } from "../../lib/cn";
import { DUR, EASE } from "../../lib/motion";
import { extractSources } from "../../lib/sources";
import { openExternal } from "../../lib/account";
import { ClarkMark } from "../ClarkMark";
import { Md, MD_CLASSES } from "../Message";
import type { ContentBlock, ToolCall } from "../../core-bridge/types";

const RESEARCH_PHASES = ["Plan", "Search", "Read", "Synthesize"] as const;

function blocksText(blocks: ContentBlock[]): string {
  return blocks.map((block) => (block.type === "text" ? block.text : `[${block.type}]`)).join("");
}

export function researchQuery(call: ToolCall): string {
  const query = (call.raw_input as { query?: string } | undefined)?.query;
  return (query || call.title.replace(/^clark_research:\s*/, "")).trim();
}

function ResearchProcess({ reduce }: { reduce: boolean | null }) {
  return (
    <div className="space-y-2.5" aria-label="Clark Cloud Agent research process">
      <div className="grid grid-cols-4 gap-1.5">
        {RESEARCH_PHASES.map((phase, index) => (
          <div key={phase} className="min-w-0">
            <div className="relative h-0.5 overflow-hidden rounded-full bg-border">
              <motion.span
                className="absolute inset-y-0 left-0 rounded-full bg-accent"
                initial={false}
                animate={
                  reduce
                    ? { width: index === 0 ? "100%" : "45%", opacity: index < 2 ? 1 : 0.45 }
                    : { width: ["20%", "100%", "20%"], opacity: [0.35, 1, 0.35] }
                }
                transition={
                  reduce
                    ? { duration: 0 }
                    : { duration: 1.8, repeat: Infinity, delay: index * 0.2, ease: "easeInOut" }
                }
              />
            </div>
            <span className="mt-1 block truncate text-[11px] text-ink-faint">{phase}</span>
          </div>
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-ink-muted">
        <span>Web search</span>
        <span className="text-border-strong" aria-hidden>·</span>
        <span>Source reading</span>
        <span className="text-border-strong" aria-hidden>·</span>
        <span>Cited synthesis</span>
      </div>
    </div>
  );
}

function ResearchSources({ call }: { call: ToolCall }) {
  const sources = extractSources(blocksText(call.content));
  if (sources.length === 0) return null;
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-1.5">
      {sources.slice(0, 4).map((source, index) => (
        <button
          key={`${source.url}-${index}`}
          type="button"
          onClick={() => void openExternal(source.url)}
          title={source.url}
          className="flex max-w-40 items-center gap-1 rounded-md bg-chip px-2 py-0.5 text-xs text-ink-secondary transition hover:bg-bg-hover hover:text-ink"
        >
          <span className="truncate">{source.label}</span>
          <ExternalLink className="size-2.5 shrink-0 text-ink-faint" />
        </button>
      ))}
      {sources.length > 4 && <span className="text-xs text-ink-faint">+{sources.length - 4} more</span>}
    </div>
  );
}

function ResearchDetail({ call }: { call: ToolCall }) {
  const findings = blocksText(call.content).trim();
  const sources = extractSources(findings);
  if (!findings) return <p className="text-xs text-ink-faint">No findings.</p>;
  return (
    <div className="space-y-3">
      <div className={cn("text-sm leading-relaxed", MD_CLASSES)}>
        <Md>{findings}</Md>
      </div>
      {sources.length > 0 && (
        <div className="border-t border-border-subtle pt-2.5">
          <div className="mb-1.5 flex items-center gap-1.5 text-xs font-medium text-ink-faint">
            <Globe2 className="size-3" /> Sources
          </div>
          <ResearchSources call={call} />
        </div>
      )}
    </div>
  );
}

function Status({ call, sourceCount }: { call: ToolCall; sourceCount: number }) {
  if (call.status === "failed") {
    return (
      <span className="flex items-center gap-1 text-xs font-medium text-danger">
        <X className="size-3" /> Failed
      </span>
    );
  }
  if (call.status === "cancelled") {
    return (
      <span className="flex items-center gap-1 text-xs font-medium text-ink-faint">
        <X className="size-3" /> Cancelled
      </span>
    );
  }
  if (call.status === "in_progress" || call.status === "pending") {
    return (
      <span className="flex items-center gap-1.5 text-xs font-medium uppercase tracking-wide text-accent">
        <Loader2 className="size-3 animate-[spin_1s_linear_infinite]" /> Live
      </span>
    );
  }
  return sourceCount > 0 ? (
    <span className="text-xs text-ink-faint">
      {sourceCount} source{sourceCount === 1 ? "" : "s"}
    </span>
  ) : (
    <span className="text-xs text-ink-faint">Complete</span>
  );
}

export function ResearchWork({ call, active }: { call: ToolCall; active: boolean }) {
  const reduce = useReducedMotion();
  const [open, setOpen] = useState(active);
  const findings = blocksText(call.content).trim();
  const sources = extractSources(findings);
  const hasFindings = findings.length > 0;
  const canOpen = active || hasFindings;

  useEffect(() => {
    if (active) setOpen(true);
  }, [active]);

  return (
    <motion.section
      id={`tool-call-${call.id}`}
      data-tool-call-id={call.id}
      initial={reduce ? false : { opacity: 0, y: 4 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: DUR.base, ease: EASE.out }}
      className="my-1 overflow-hidden rounded-xl border border-border bg-bg-secondary/55 shadow-soft"
      aria-label="Clark Cloud Agent"
    >
      <div className="h-0.5 bg-accent/80" aria-hidden />
      <button
        type="button"
        onClick={() => canOpen && setOpen((value) => !value)}
        disabled={!canOpen}
        aria-expanded={open}
        className={cn(
          "group flex w-full items-center gap-2.5 px-3 py-2.5 text-left",
          canOpen && "cursor-pointer transition hover:bg-bg-hover/45",
        )}
      >
        <ClarkMark size={30} className="shrink-0" />
        <span className="min-w-0 flex-1">
          <span className="block text-sm font-medium leading-5 text-ink">Clark Cloud Agent</span>
          <span className="block truncate text-xs leading-4 text-ink-faint">
            Running securely on clarkchat.com
          </span>
        </span>
        <span className="flex shrink-0 items-center gap-2">
          <Status call={call} sourceCount={sources.length} />
          {canOpen && (
            <ChevronRight
              className={cn("size-3.5 text-ink-faint transition", open && "rotate-90")}
              aria-hidden
            />
          )}
        </span>
      </button>

      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            initial={reduce ? false : { height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={reduce ? { opacity: 0 } : { height: 0, opacity: 0 }}
            transition={{ duration: DUR.fast, ease: EASE.inOut }}
            className="overflow-hidden border-t border-border-subtle"
          >
            <div className="space-y-3 px-3 py-2.5">
              <p className="truncate text-sm text-ink-secondary" title={researchQuery(call)}>
                {researchQuery(call)}
              </p>
              {active && !hasFindings ? (
                <ResearchProcess reduce={reduce} />
              ) : (
                <div className="max-h-64 overflow-auto pr-1">
                  <ResearchDetail call={call} />
                </div>
              )}
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {!open && hasFindings && (
        <button
          type="button"
          onClick={() => setOpen(true)}
          className="flex w-full items-center justify-between gap-3 border-t border-border-subtle px-3 py-2 text-left transition hover:bg-bg-hover/45"
        >
          <span className="min-w-0 truncate text-xs text-ink-muted">{researchQuery(call)}</span>
          <span className="shrink-0 text-xs font-medium text-accent">View research brief</span>
        </button>
      )}
    </motion.section>
  );
}
