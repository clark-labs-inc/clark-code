import { useEffect, useRef, useState } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import {
  Bot,
  ChevronDown,
  ChevronRight,
  Circle,
  CircleCheck,
  ExternalLink,
  Globe2,
  Loader2,
  TriangleAlert,
  X,
} from "lucide-react";
import { cn } from "../../lib/cn";
import {
  DUR,
  EASE,
  REDUCED_EXIT,
  RISE_SMALL,
  accessibleMotion,
} from "../../lib/motion";
import { extractSources } from "../../lib/sources";
import { openExternal } from "../../lib/account";
import { ProductMark } from "../ProductMark";
import { MarkdownContent, MARKDOWN_CLASSES } from "../MarkdownContent";
import type {
  ContentBlock,
  ToolCall,
  ToolCallProgress,
  ToolProgressAgent,
  ToolProgressPhase,
  ToolStatus,
} from "../../core-bridge/types";

function blocksText(blocks: ContentBlock[]): string {
  return blocks.map((block) => (block.type === "text" ? block.text : `[${block.type}]`)).join("");
}

export function researchQuery(call: ToolCall): string {
  const query = (call.raw_input as { query?: string } | undefined)?.query;
  return (query || call.title.replace(/^[a-z][a-z0-9_]*:\s*/, "")).trim();
}

const STATUS_LABEL: Record<ToolStatus, string> = {
  pending: "Pending",
  in_progress: "Running",
  completed: "Complete",
  cancelled: "Cancelled",
  failed: "Failed",
};

const STATUS_TEXT: Record<ToolStatus, string> = {
  pending: "text-ink-faint",
  in_progress: "text-accent",
  completed: "text-success",
  cancelled: "text-ink-faint",
  failed: "text-danger",
};

function ProgressIcon({
  status,
  className,
}: {
  status: ToolStatus;
  className?: string;
}) {
  if (status === "completed") {
    return <CircleCheck aria-hidden className={cn("text-success", className)} />;
  }
  if (status === "in_progress") {
    return (
      <Loader2
        aria-hidden
        className={cn("animate-[spin_1s_linear_infinite] text-accent", className)}
      />
    );
  }
  if (status === "failed") {
    return <TriangleAlert aria-hidden className={cn("text-danger", className)} />;
  }
  if (status === "cancelled") {
    return <X aria-hidden className={cn("text-ink-faint", className)} />;
  }
  return <Circle aria-hidden className={cn("text-ink-faint", className)} />;
}

function Phase({ phase }: { phase: ToolProgressPhase }) {
  const current = phase.status === "in_progress";
  return (
    <li className="border-t border-border-subtle first:border-t-0">
      <div className="grid min-h-8 grid-cols-[0.75rem_1rem_minmax(0,1fr)_auto] items-center gap-2 px-0.5 text-sm">
        {current ? (
          <ChevronDown aria-hidden className="size-3 text-ink-faint" />
        ) : (
          <span aria-hidden />
        )}
        <ProgressIcon status={phase.status} className="size-4" />
        <span className={cn("min-w-0 truncate", current ? "font-medium text-ink" : "text-ink-secondary")}>
          {phase.title}
        </span>
        <span className={cn("pl-4 text-xs font-medium", STATUS_TEXT[phase.status])}>
          {STATUS_LABEL[phase.status]}
        </span>
      </div>

      {current && phase.steps.length > 0 && (
        <ol className="relative ml-[1.7rem] border-l border-accent/45 pb-1 pl-4">
          {phase.steps.map((step) => (
            <li
              key={step.id}
              className="grid min-h-7 grid-cols-[1rem_minmax(0,1fr)_auto] items-center gap-2 pr-0.5 text-sm"
            >
              <ProgressIcon status={step.status} className="size-3.5" />
              <span className="min-w-0">
                <span className="block truncate text-ink-muted">{step.title}</span>
                {step.summary && step.status === "in_progress" && (
                  <span className="block truncate text-xs text-ink-faint">{step.summary}</span>
                )}
              </span>
              <span className={cn("pl-4 text-xs", STATUS_TEXT[step.status])}>
                {STATUS_LABEL[step.status]}
              </span>
            </li>
          ))}
        </ol>
      )}
    </li>
  );
}

function Agent({ agent }: { agent: ToolProgressAgent }) {
  const detail = agent.activity || agent.summary || STATUS_LABEL[agent.status];
  return (
    <li className="grid min-h-8 grid-cols-[1rem_minmax(0,1fr)_auto] items-center gap-2 border-t border-border-subtle px-0.5 text-sm sm:grid-cols-[1rem_minmax(0,1fr)_auto_minmax(12rem,1.25fr)]">
      <Bot aria-hidden className="size-3.5 text-ink-faint" />
      <span className="min-w-0 truncate text-ink-muted">{agent.label}</span>
      <span className={cn("flex items-center gap-1.5 pl-3 text-xs font-medium", STATUS_TEXT[agent.status])}>
        <ProgressIcon status={agent.status} className="size-3.5" />
        {STATUS_LABEL[agent.status]}
      </span>
      <span className="col-span-2 min-w-0 truncate pb-1 pl-6 text-xs text-ink-faint sm:col-span-1 sm:pb-0 sm:pl-6">
        {detail}
      </span>
    </li>
  );
}

export function ResearchOutline({
  progress,
}: {
  progress?: ToolCallProgress;
}) {
  if (!progress || (progress.phases.length === 0 && progress.agents.length === 0)) {
    return (
      <div className="flex min-h-9 items-center gap-2 text-sm text-ink-muted" aria-label="Research agent progress">
        <ProgressIcon status="in_progress" className="size-4" />
        <span>{progress?.latest_activity || "Starting research agent"}</span>
      </div>
    );
  }

  return (
    <div className="max-h-72 overflow-y-auto pr-1" aria-label="Research agent progress">
      <ol className="-ml-5">{progress.phases.map((phase) => <Phase key={phase.id} phase={phase} />)}</ol>
      {progress.agents.length > 0 && (
        <section className="mt-1" aria-label="Parallel research agents">
          <h4 className="min-h-8 border-t border-border-subtle px-0.5 pt-2 text-xs font-medium text-ink-faint">
            Parallel research · {progress.agents.length} agent{progress.agents.length === 1 ? "" : "s"}
          </h4>
          <ul>{progress.agents.map((agent) => <Agent key={agent.id} agent={agent} />)}</ul>
        </section>
      )}
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
      <div className={cn("text-sm leading-relaxed", MARKDOWN_CLASSES)}>
        <MarkdownContent>{findings}</MarkdownContent>
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
    return <span className="flex items-center gap-1 text-xs font-medium text-danger"><X className="size-3" /> Failed</span>;
  }
  if (call.status === "cancelled") {
    return <span className="flex items-center gap-1 text-xs font-medium text-ink-faint"><X className="size-3" /> Cancelled</span>;
  }
  if (call.status === "in_progress" || call.status === "pending") {
    return (
      <span className="flex items-center gap-1.5 text-xs font-medium uppercase tracking-wide text-accent">
        <Loader2 className="size-3 animate-[spin_1s_linear_infinite]" /> Live
      </span>
    );
  }
  return sourceCount > 0 ? (
    <span className="text-xs text-ink-faint">{sourceCount} source{sourceCount === 1 ? "" : "s"}</span>
  ) : (
    <span className="text-xs text-ink-faint">Complete</span>
  );
}

function completedSubtitle(call: ToolCall): string {
  if (call.status === "failed") return call.progress?.latest_activity || "Research failed";
  if (call.status === "cancelled") return "Research cancelled";
  if (call.status === "completed") return "Research complete";
  return call.progress?.latest_activity || "Starting research agent";
}

export function ResearchWork({ call, active }: { call: ToolCall; active: boolean }) {
  const reduce = useReducedMotion();
  const [open, setOpen] = useState(active);
  const [activityOpen, setActivityOpen] = useState(false);
  const wasActive = useRef(active);
  const findings = blocksText(call.content).trim();
  const sources = extractSources(findings);
  const hasFindings = findings.length > 0;
  const canOpen = active || hasFindings || Boolean(call.progress);

  useEffect(() => {
    if (active) setOpen(true);
    else if (wasActive.current) setOpen(false);
    wasActive.current = active;
  }, [active]);

  return (
    <m.section
      id={`tool-call-${call.id}`}
      data-tool-call-id={call.id}
      {...accessibleMotion(RISE_SMALL, reduce)}
      className="mb-1 mt-3 overflow-hidden rounded-xl border border-border-subtle bg-bg-secondary/45"
      aria-label="Research agent"
    >
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
        <ProductMark size={30} className="shrink-0" />
        <span className="min-w-0 flex-1">
          <span className="block text-sm font-medium leading-5 text-ink">Research agent</span>
          <span aria-live="polite" className="block truncate text-xs leading-4 text-ink-faint">
            {completedSubtitle(call)}
          </span>
        </span>
        <span className="flex shrink-0 items-center gap-2">
          <Status call={call} sourceCount={sources.length} />
          {canOpen && (
            <ChevronDown className={cn("size-3.5 text-ink-faint transition", open && "rotate-180")} aria-hidden />
          )}
        </span>
      </button>

      <AnimatePresence initial={false}>
        {open && (
          <m.div
            initial={reduce ? false : { height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={reduce ? REDUCED_EXIT : { height: 0, opacity: 0 }}
            transition={{ duration: reduce ? 0 : DUR.fast, ease: EASE.inOut }}
            className="overflow-hidden border-t border-border-subtle"
          >
            <div className="space-y-2 px-3 py-2.5">
              <p className="truncate text-sm text-ink-secondary" title={researchQuery(call)}>{researchQuery(call)}</p>
              {active || !hasFindings ? (
                <ResearchOutline progress={call.progress} />
              ) : (
                <>
                  <div className="max-h-64 overflow-auto pr-1"><ResearchDetail call={call} /></div>
                  {call.progress && (
                    <div className="border-t border-border-subtle pt-2">
                      <button
                        type="button"
                        onClick={() => setActivityOpen((value) => !value)}
                        aria-expanded={activityOpen}
                        className="flex w-full items-center justify-between text-xs font-medium text-ink-muted transition hover:text-ink-secondary"
                      >
                        Run activity
                        <ChevronRight className={cn("size-3.5 transition", activityOpen && "rotate-90")} />
                      </button>
                      <AnimatePresence initial={false}>
                        {activityOpen && (
                          <m.div
                            initial={reduce ? false : { height: 0, opacity: 0 }}
                            animate={{ height: "auto", opacity: 1 }}
                            exit={reduce ? REDUCED_EXIT : { height: 0, opacity: 0 }}
                            transition={{ duration: reduce ? 0 : DUR.fast, ease: EASE.inOut }}
                            className="overflow-hidden pt-2"
                          >
                            <ResearchOutline progress={call.progress} />
                          </m.div>
                        )}
                      </AnimatePresence>
                    </div>
                  )}
                </>
              )}
            </div>
          </m.div>
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
    </m.section>
  );
}
