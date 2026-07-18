import { memo, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { ArrowDown, X } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { currentActivity, shouldShowPending } from "../lib/activity";
import { humanizeError, humanizeRunFailure } from "../lib/errors";
import { cn } from "../lib/cn";
import { DUR, EASE, INSTANT } from "../lib/motion";
import { Message } from "./Message";
import { WorkBlock } from "./work/WorkBlock";
import { ArtifactCard } from "./work/ArtifactCard";
import { PermissionGate } from "./PermissionGate";
import { UpgradePrompt } from "./UpgradePrompt";
import { FanOutPanel } from "./FanOutPanel";
import { PlanChecklist } from "./PlanChecklist";
import type { Artifact, TimelineItem, ToolCall } from "../core-bridge/types";

/** A row of pulsing dots — the model is generating. Memoized so its animation
 *  isn't re-evaluated on every streamed-token re-render of the parent. */
const Dots = memo(function Dots() {
  return (
    <span className="flex items-center gap-[3px]" aria-hidden>
      {[0, 1, 2].map((i) => (
        <motion.span
          key={i}
          className="size-1.5 rounded-full bg-accent"
          animate={{ opacity: [0.3, 1, 0.3] }}
          transition={{ duration: 1.1, repeat: Infinity, delay: i * 0.18 }}
        />
      ))}
    </span>
  );
});

/** Skeleton render-preview of the assistant reply that's still streaming. */
const ReplySkeleton = memo(function ReplySkeleton() {
  return (
    <div className="space-y-2.5" aria-hidden>
      <div className="skeleton h-3.5 w-[92%]" />
      <div className="skeleton h-3.5 w-[84%]" />
      <div className="skeleton h-3.5 w-[64%]" />
    </div>
  );
});

/** "Working now" — dots + label, plus a skeleton preview before the first
 *  tokens of the reply arrive. Hidden while a tool line shows its own spinner. */
function Pending({ label, detail, skeleton }: { label: string; detail?: string; skeleton: boolean }) {
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2.5 text-sm text-ink-muted">
        <Dots />
        <span className="truncate">
          {label || "Thinking…"}
          {detail && <span className="ml-1.5 font-mono text-xs text-ink-faint">{detail}</span>}
        </span>
      </div>
      {skeleton && <ReplySkeleton />}
    </div>
  );
}

/** Group consecutive tool-call lines so agent "work" reads as a dense block. */
type Block =
  | { kind: "item"; item: TimelineItem; timelineIndex: number; key: string }
  | { kind: "work"; ids: string[]; key: string };

function group(timeline: TimelineItem[]): Block[] {
  const blocks: Block[] = [];
  timeline.forEach((item, i) => {
    if (item.item === "tool_call") {
      const last = blocks[blocks.length - 1];
      if (last && last.kind === "work") last.ids.push(item.id);
      else blocks.push({ kind: "work", ids: [item.id], key: `w${i}` });
    } else {
      blocks.push({ kind: "item", item, timelineIndex: i, key: `i${i}` });
    }
  });
  return blocks;
}

/** Common motion props for transient elements at the foot of the conversation.
 *  Enter: fade + gentle rise with height opening, so the content below doesn't
 *  snap down. Exit: fade + collapse, so when one banner replaces another (or
 *  clears) the list reflows smoothly instead of jumping. */
const TRANSIENT = {
  initial: { opacity: 0, y: 6, height: 0 },
  animate: { opacity: 1, y: 0, height: "auto" },
  exit: { opacity: 0, height: 0, transition: { duration: DUR.base, ease: EASE.inOut } },
  transition: { duration: DUR.base, ease: EASE.out },
  style: { overflow: "hidden" },
};

/** Reduced-motion variant: truly instant — no opacity keyframe (a fading frame
 *  still paints partial opacity in WKWebView, which reads as flicker). */
const TRANSIENT_INSTANT = INSTANT;

// `min-w-0` lets this flex child shrink to the column width (flex items default
// to min-width:auto, so an unbreakable token — a long URL or a raw provider JSON
// blob — would otherwise grow the box past the column); `break-words` +
// `whitespace-pre-wrap` then wrap that token inside the border instead of
// spilling past its right edge.
const DANGER_BANNER =
  "min-w-0 whitespace-pre-wrap break-words rounded-lg border border-danger/40 bg-danger/8 px-3.5 py-2.5 text-sm text-danger";

/** Small dismiss (×) affordance for the error banners. */
function DismissButton({ onClick }: { onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      aria-label="Dismiss"
      className="-mr-1 -mt-0.5 grid size-6 shrink-0 place-items-center rounded-md text-danger/70 transition hover:bg-danger/10 hover:text-danger"
    >
      <X className="size-3.5" />
    </button>
  );
}

/** How many timeline blocks render before older history collapses behind a
 *  "Show earlier" control. Generous enough that normal sessions never notice. */
const TIMELINE_WINDOW = 80;

export function Conversation({
  activeArtifactId,
  onOpenArtifact,
}: {
  activeArtifactId?: string | null;
  onOpenArtifact?: (artifact: Artifact) => void;
}) {
  const reduce = useReducedMotion();
  const snapshot = useSessionStore((s) => s.snapshot);
  const session = useSessionStore((s) => s.session);
  const error = useSessionStore((s) => s.error);
  const dismissError = useSessionStore((s) => s.dismissError);
  const dismissFailedRun = useSessionStore((s) => s.dismissFailedRun);
  const dismissedFailedRuns = useSessionStore((s) => s.dismissedFailedRuns);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [showAll, setShowAll] = useState(false);
  // Collapse history again when switching conversations.
  useEffect(() => setShowAll(false), [session?.id]);
  // Pin to the bottom only when the user is already there — never yank them up
  // while they're reading scrollback. Instant (not smooth) keeps streaming stable.
  const stuck = useRef(true);
  const [atBottom, setAtBottom] = useState(true);
  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const bottom = el.scrollHeight - el.scrollTop - el.clientHeight < 96;
    stuck.current = bottom;
    if (bottom !== atBottom) setAtBottom(bottom);
  };
  const scrollToBottom = () => {
    const el = scrollRef.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: reduce ? "auto" : "smooth" });
  };

  const { timeline, tool_calls: toolCalls, artifacts, runs, pending_permission, plan } = snapshot;

  useEffect(() => {
    const el = scrollRef.current;
    if (el && stuck.current) el.scrollTop = el.scrollHeight;
  }, [timeline, toolCalls]);

  if (!session) return null;

  const visible = timeline;
  const allBlocks = group(visible);
  // Long transcripts: render only the recent window. A 400-item DOM makes every
  // style/layout pass (and each streamed frame) pay for history the user isn't
  // reading — the dominant cost on slower machines. "Show earlier" reveals all.
  const windowed = !showAll && allBlocks.length > TIMELINE_WINDOW;
  const blocks = windowed ? allBlocks.slice(allBlocks.length - TIMELINE_WINDOW) : allBlocks;
  const hiddenCount = allBlocks.length - blocks.length;
  const lastBlockKey = blocks[blocks.length - 1]?.key;

  const activity = currentActivity(snapshot);
  const last = visible[visible.length - 1];
  const awaitingReply = !last || (last.item === "message" && last.role === "user");
  // This placeholder owns only the gap before the first agent response. Typed
  // agent content (including reasoning) and tool rows own their live state.
  const showPending = shouldShowPending(snapshot);
  // The "Run failed" banner reflects only the MOST RECENT run — so it clears
  // on its own once the next turn starts, instead of every past failure
  // lingering below the messages forever. It can also be dismissed outright.
  const runList = Object.values(runs);
  const latestRun = runList[runList.length - 1];
  const failed =
    latestRun?.status === "failed" && !dismissedFailedRuns.includes(latestRun.id)
      ? latestRun
      : undefined;
  const outOfCredits = failed?.outcome?.failure_kind === "insufficient_credits";
  return (
    <div ref={scrollRef} onScroll={onScroll} className="flex-1 overflow-y-auto">
      <div className="mx-auto flex max-w-2xl flex-col gap-4 px-5 py-5">
        {visible.length === 0 && !showPending && (
          <p className="py-10 text-center text-sm text-ink-faint">
            Ask Clark anything — file work, web research, and computer use show up here as it works.
          </p>
        )}

        {windowed && (
          <button
            onClick={() => setShowAll(true)}
            className="mx-auto rounded-full border border-border-subtle bg-bg-elevated px-3.5 py-1.5 text-xs font-medium text-ink-muted transition hover:bg-bg-hover hover:text-ink-secondary"
          >
            Show {hiddenCount} earlier item{hiddenCount === 1 ? "" : "s"}
          </button>
        )}

        {blocks.map((block) => {
          if (block.kind === "work") {
            // Codex form: a quiet stack of inline tool lines, no card border.
            const calls = block.ids
              .map((id) => toolCalls[id])
              .filter(Boolean) as ToolCall[];
            return <WorkBlock key={block.key} calls={calls} />;
          }
          const { item } = block;
          if (item.item === "message")
            return (
              <Message
                key={block.key}
                role={item.role}
                blocks={item.blocks}
                timelineIndex={block.timelineIndex}
                streaming={activity.busy && block.key === lastBlockKey && item.role === "agent"}
              />
            );
          if (item.item === "artifact") {
            const a = artifacts.find((x) => x.id === item.id);
            return a ? (
              <div
                id={`artifact-${a.id}`}
                key={block.key}
                tabIndex={-1}
                className={cn(
                  "relative outline-none focus-visible:ring-2 focus-visible:ring-accent",
                  a.id === activeArtifactId &&
                    "after:absolute after:left-full after:top-1/2 after:h-px after:w-5 after:bg-accent after:content-['']",
                )}
              >
                <ArtifactCard
                  artifact={a}
                  active={a.id === activeArtifactId}
                  onOpen={onOpenArtifact}
                />
              </div>
            ) : null;
          }
          if (item.item === "plan") {
            return <PlanChecklist key={block.key} plan={item.plan ?? plan} />;
          }
          return null;
        })}

        <FanOutPanel />

        {/* Default (sync) mode, not popLayout: popLayout yanks an exiting
            banner OUT of the layout flow, so a collapsing Pending briefly
            floats over the content below it. In-flow exit collapses height in
            place — no overlap. */}
        <AnimatePresence initial={false}>
          {showPending && (
            <motion.div key="pending" {...(reduce ? TRANSIENT_INSTANT : TRANSIENT)}>
              <Pending label={activity.label} detail={activity.detail} skeleton={awaitingReply} />
            </motion.div>
          )}
          {pending_permission && (
            <motion.div key="permission" {...(reduce ? TRANSIENT_INSTANT : TRANSIENT)}>
              <PermissionGate req={pending_permission} />
            </motion.div>
          )}
          {failed && outOfCredits && (
            <motion.div key="upgrade" {...(reduce ? TRANSIENT_INSTANT : TRANSIENT)}>
              <UpgradePrompt />
            </motion.div>
          )}
          {failed && !outOfCredits && (
            <motion.div
              key="failed"
              {...(reduce ? TRANSIENT_INSTANT : TRANSIENT)}
              className={cn(DANGER_BANNER, "flex items-start gap-2")}
              title={failed.outcome?.error || undefined}
            >
              <div className="min-w-0 flex-1">
                <span className="font-medium">Run failed.</span>{" "}
                {humanizeRunFailure(failed.outcome)}
              </div>
              <DismissButton onClick={() => dismissFailedRun(failed.id)} />
            </motion.div>
          )}
          {error && (
            <motion.div
              key="error"
              {...(reduce ? TRANSIENT_INSTANT : TRANSIENT)}
              className={cn(DANGER_BANNER, "flex items-start gap-2")}
              title={error}
            >
              <div className="min-w-0 flex-1">{humanizeError(error)}</div>
              <DismissButton onClick={dismissError} />
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      {/* Jump-to-latest: a sticky pill (stays in the scroll flow — no positioned
          ancestor) shown only when the user has scrolled up during/after a run. */}
      <AnimatePresence>
        {!atBottom && visible.length > 0 && (
          <motion.button
            onClick={scrollToBottom}
            initial={reduce ? false : { opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={reduce ? { opacity: 0, transition: { duration: 0 } } : { opacity: 0, y: 8 }}
            transition={{ duration: DUR.fast, ease: EASE.out }}
            className="sticky bottom-4 left-1/2 z-10 mx-auto flex w-fit -translate-x-1/2 items-center gap-1.5 rounded-full bg-bg-elevated px-3 py-1.5 text-xs font-medium text-ink-secondary shadow-lg ring-1 ring-border-subtle transition-colors hover:text-ink"
          >
            <ArrowDown className="size-3.5" /> Jump to latest
          </motion.button>
        )}
      </AnimatePresence>
    </div>
  );
}
