import { memo, useEffect, useLayoutEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { ArrowDown, X } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { currentActivity, shouldShowPending } from "../lib/activity";
import { humanizeError, humanizeRunFailure } from "../lib/errors";
import { cn } from "../lib/cn";
import { DUR, EASE, INSTANT } from "../lib/motion";
import {
  conversationScrollTarget,
  isConversationAtBottom,
  isConversationScrollUp,
  shouldFollowConversation,
  type ConversationScrollState,
} from "../lib/conversationScroll";
import { Message } from "./Message";
import { WorkBlock } from "./work/WorkBlock";
import { ArtifactCard } from "./work/ArtifactCard";
import { PermissionGate } from "./PermissionGate";
import { UpgradePrompt } from "./UpgradePrompt";
import { FanOutPanel } from "./FanOutPanel";
import { ExecutionChecklistCard } from "./PlanChecklist";
import { ProposedPlanCard } from "./ProposedPlanCard";
import { SideQuestionCard } from "./SideQuestionCard";
import { GoalWorkSummary } from "./GoalWorkSummary";
import { ProviderIncidentCard } from "./ProviderIncidentCard";
import type { Artifact, GoalState, TimelineItem, ToolCall } from "../core-bridge/types";
import { effectiveModelSettings, isIncludedCodingModel } from "../lib/localAgent";
import { isIncludedWeeklyAllowanceExhausted } from "../lib/errors";

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
type BaseBlock =
  | { kind: "item"; item: TimelineItem; timelineIndex: number; key: string }
  | { kind: "work"; ids: string[]; run?: string; key: string };
type Block = BaseBlock | { kind: "goal_work"; blocks: BaseBlock[]; key: string };

function group(timeline: TimelineItem[], goal?: GoalState): Block[] {
  const base: BaseBlock[] = [];
  timeline.forEach((item, i) => {
    if (item.item === "tool_call") {
      const last = base[base.length - 1];
      if (last && last.kind === "work" && last.run === item.run) last.ids.push(item.id);
      else base.push({ kind: "work", ids: [item.id], run: item.run, key: `w${i}` });
    } else {
      base.push({ kind: "item", item, timelineIndex: i, key: `i${i}` });
    }
  });

  if (!goal?.run) return base;
  const blocks: Block[] = [];
  for (const block of base) {
    const belongsToGoal = block.kind === "work"
      ? block.run === goal.run
      : block.item.item === "execution_checklist" || block.item.item === "proposed_plan"
        ? block.item.run === goal.run
        : block.item.item === "message"
          ? block.item.run === goal.run && (
              block.item.role === "system" ||
              (block.item.role === "agent" && block.item.phase !== "final_answer")
            )
          : false;
    if (!belongsToGoal) {
      blocks.push(block);
      continue;
    }
    const last = blocks[blocks.length - 1];
    if (last?.kind === "goal_work") last.blocks.push(block);
    else blocks.push({ kind: "goal_work", blocks: [block], key: `g${block.key}` });
  }
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
const WARNING_BANNER =
  "min-w-0 whitespace-pre-wrap break-words rounded-lg border border-warning/30 bg-warning/10 px-3.5 py-2.5 text-sm text-ink-muted";
const STOPPED_BANNER =
  "min-w-0 whitespace-pre-wrap break-words rounded-lg border border-border bg-bg-secondary px-3.5 py-2.5 text-sm text-ink-muted";

/** Small dismiss (×) affordance for the error banners. */
function DismissButton({ onClick, muted = false }: { onClick: () => void; muted?: boolean }) {
  return (
    <button
      onClick={onClick}
      aria-label="Dismiss"
      className={cn(
        "-mr-1 -mt-0.5 grid size-6 shrink-0 place-items-center rounded-md transition",
        muted
          ? "text-ink-faint hover:bg-bg-hover hover:text-ink-muted"
          : "text-danger hover:bg-danger/10",
      )}
    >
      <X className="size-3.5" />
    </button>
  );
}

/** How many timeline blocks render before older history collapses behind a
 *  "Show earlier" control. Generous enough that normal sessions never notice. */
const TIMELINE_WINDOW = 80;

/** Conversation is intentionally kept mounted while live sessions switch, so
 * the scroll element is shared. Keep its viewport state keyed by conversation
 * instead of leaking one chat's pinned/scrollback state into the next. Module
 * scope also preserves it across the loading screen used for cold reopens. */
const scrollByConversation = new Map<string, ConversationScrollState>();

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
  const includedModel = useSessionStore((s) =>
    isIncludedCodingModel(
      effectiveModelSettings(s.localSettings, s.chatModels, s.session?.id ?? null).model,
    ),
  );
  const error = useSessionStore((s) => s.error);
  const dismissError = useSessionStore((s) => s.dismissError);
  const dismissFailedRun = useSessionStore((s) => s.dismissFailedRun);
  const dismissedFailedRuns = useSessionStore((s) => s.dismissedFailedRuns);
  const sessionId = session?.id;
  const scrollRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const lastScrollTop = useRef(0);
  const scrollingToBottom = useRef(false);
  const upwardWheel = useRef(false);
  const [showAll, setShowAll] = useState(false);
  const activity = currentActivity(snapshot);
  // Collapse history again when switching conversations.
  useEffect(() => setShowAll(false), [sessionId]);
  // Pin to the bottom only when the user is already there — never yank them up
  // while they're reading scrollback. Instant (not smooth) keeps streaming stable.
  const stuck = useRef(true);
  const [atBottom, setAtBottom] = useState(true);
  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const previousScrollTop = lastScrollTop.current;
    const nearBottom = isConversationAtBottom(el.scrollHeight, el.scrollTop, el.clientHeight);
    const movedUp = isConversationScrollUp(previousScrollTop, el.scrollTop);
    const userScrolledUp = upwardWheel.current && movedUp;
    upwardWheel.current = false;
    const following = shouldFollowConversation(
      previousScrollTop,
      el.scrollTop,
      nearBottom,
      scrollingToBottom.current,
      userScrolledUp,
    );
    lastScrollTop.current = el.scrollTop;
    if (movedUp || nearBottom) scrollingToBottom.current = false;
    stuck.current = following;
    if (sessionId) {
      scrollByConversation.set(sessionId, { scrollTop: el.scrollTop, atBottom: following });
    }
    if (following !== atBottom) setAtBottom(following);
  };
  const noteUpwardWheel = (deltaY: number) => {
    // Record intent; wait for an actual scroll event before changing state. An
    // endpoint bounce can emit a negative wheel delta without moving the
    // transcript, and should not summon a stale "Jump to latest" button.
    if (deltaY < 0) upwardWheel.current = true;
  };
  const scrollToBottom = () => {
    const el = scrollRef.current;
    if (el) {
      scrollingToBottom.current = true;
      upwardWheel.current = false;
      stuck.current = true;
      setAtBottom(true);
      el.scrollTo({ top: el.scrollHeight, behavior: reduce ? "auto" : "smooth" });
      if (sessionId) {
        scrollByConversation.set(sessionId, { scrollTop: el.scrollHeight, atBottom: true });
      }
    }
  };

  const {
    timeline,
    tool_calls: toolCalls,
    artifacts,
    runs,
    pending_permission,
    execution_checklist,
    proposed_plan,
    goal,
    provider_incidents: providerIncidents,
  } = snapshot;

  // Restore after React has committed the target transcript but before paint,
  // avoiding a frame at the previous conversation's unrelated scrollTop.
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el || !sessionId) return;
    const remembered = scrollByConversation.get(sessionId);
    const busy = currentActivity(useSessionStore.getState().snapshot).busy;
    el.scrollTop = conversationScrollTarget(remembered, busy, el.scrollHeight);
    lastScrollTop.current = el.scrollTop;
    scrollingToBottom.current = false;
    const bottom = isConversationAtBottom(el.scrollHeight, el.scrollTop, el.clientHeight);
    stuck.current = bottom;
    setAtBottom(bottom);
    scrollByConversation.set(sessionId, { scrollTop: el.scrollTop, atBottom: bottom });
  }, [sessionId]);

  useEffect(() => {
    const el = scrollRef.current;
    if (el && stuck.current) {
      el.scrollTop = el.scrollHeight;
      lastScrollTop.current = el.scrollTop;
      if (sessionId) {
        scrollByConversation.set(sessionId, { scrollTop: el.scrollTop, atBottom: true });
      }
    }
  }, [sessionId, timeline, toolCalls]);

  // Timeline rows, images, and animated pending/permission banners can change
  // height after their snapshot render. Follow the actual content box while
  // pinned so "latest" remains truly visible rather than a few pixels below
  // the viewport after an enter animation settles.
  useEffect(() => {
    const el = scrollRef.current;
    const content = contentRef.current;
    if (!el || !content || !sessionId || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      if (!stuck.current) return;
      el.scrollTop = el.scrollHeight;
      lastScrollTop.current = el.scrollTop;
      scrollByConversation.set(sessionId, { scrollTop: el.scrollTop, atBottom: true });
    });
    observer.observe(content);
    return () => observer.disconnect();
  }, [sessionId]);

  if (!session) return null;

  const visible = timeline;
  const allBlocks = group(visible, goal);
  // Long transcripts: render only the recent window. A 400-item DOM makes every
  // style/layout pass (and each streamed frame) pay for history the user isn't
  // reading — the dominant cost on slower machines. "Show earlier" reveals all.
  const windowed = !showAll && allBlocks.length > TIMELINE_WINDOW;
  const blocks = windowed ? allBlocks.slice(allBlocks.length - TIMELINE_WINDOW) : allBlocks;
  const hiddenCount = allBlocks.length - blocks.length;
  const last = visible[visible.length - 1];
  const awaitingReply = !last || (last.item === "message" && last.role === "user");
  // Tool rows and actively streaming unphased responses own their live state;
  // completed commentary keeps this row visible until the run advances or ends.
  const showPending = shouldShowPending(snapshot);
  // The "Run failed" banner reflects only the MOST RECENT run — so it clears
  // on its own once the next turn starts, instead of every past failure
  // lingering below the messages forever. It can also be dismissed outright.
  const runList = Object.values(runs);
  const latestRun = runList[runList.length - 1];
  const goalRunStatus = goal?.run ? runs[goal.run]?.status : undefined;
  const goalRunActive = goalRunStatus === "queued" || goalRunStatus === "running";
  const failed =
    latestRun?.status === "failed" && !dismissedFailedRuns.includes(latestRun.id)
      ? latestRun
      : undefined;
  const stopped =
    latestRun?.status === "cancelled" && !dismissedFailedRuns.includes(latestRun.id)
      ? latestRun
      : undefined;
  const outOfCredits = failed?.outcome?.failure_kind === "insufficient_credits";
  const includedWeeklyAllowanceExhausted = isIncludedWeeklyAllowanceExhausted(failed?.outcome);
  const interrupted = failed?.outcome?.failure_kind === "runtime_interrupted" ? failed : undefined;
  const verificationIncomplete =
    failed?.outcome?.failure_kind === "verification_incomplete" ? failed : undefined;

  const renderBlock = (block: Block | BaseBlock) => {
    if (block.kind === "goal_work") {
      return goal ? (
        <GoalWorkSummary key={block.key} goal={goal} runActive={goalRunActive}>
          {block.blocks.map(renderBlock)}
        </GoalWorkSummary>
      ) : null;
    }
    if (block.kind === "work") {
      const calls = block.ids
        .map((id) => toolCalls[id])
        .filter(Boolean) as ToolCall[];
      return <WorkBlock key={block.key} calls={calls} />;
    }
    const { item } = block;
    if (item.item === "message") {
      return (
        <Message
          key={block.key}
          role={item.role}
          blocks={item.blocks}
          phase={item.phase}
          timelineIndex={block.timelineIndex}
          streaming={
            activity.busy &&
            block.timelineIndex === visible.length - 1 &&
            item.role === "agent"
          }
        />
      );
    }
    if (item.item === "artifact") {
      const artifact = artifacts.find((candidate) => candidate.id === item.id);
      return artifact ? (
        <div
          id={`artifact-${artifact.id}`}
          key={block.key}
          tabIndex={-1}
          className={cn(
            "relative outline-none focus-visible:ring-2 focus-visible:ring-accent",
            artifact.id === activeArtifactId &&
              "after:absolute after:left-full after:top-1/2 after:h-px after:w-5 after:bg-accent after:content-['']",
          )}
        >
          <ArtifactCard
            artifact={artifact}
            active={artifact.id === activeArtifactId}
            onOpen={onOpenArtifact}
          />
        </div>
      ) : null;
    }
    if (item.item === "provider_incident") {
      const incident = providerIncidents[item.id];
      const canContinue = block.timelineIndex === timeline.length - 1
        && (incident?.status === "failed" || incident?.status === "interrupted");
      return incident ? (
        <ProviderIncidentCard
          key={block.key}
          incident={incident}
          executionLocation={session.environment?.remote ? "your remote host" : "this computer"}
          modelRouteLabel={session.provider === "local" ? "Clark's cloud model gateway" : "the selected model provider"}
          onContinue={canContinue
            ? () => void useSessionStore.getState().continueProviderIncident(incident.id)
            : undefined}
        />
      ) : null;
    }
    if (item.item === "execution_checklist") {
      return <ExecutionChecklistCard key={block.key} checklist={item.checklist ?? execution_checklist} />;
    }
    if (item.item === "proposed_plan") {
      return <ProposedPlanCard key={block.key} plan={item.plan ?? proposed_plan} />;
    }
    return null;
  };

  return (
    <div className="relative flex min-h-0 flex-1">
      <div
        ref={scrollRef}
        onScroll={onScroll}
        onWheel={(event) => noteUpwardWheel(event.deltaY)}
        className="min-h-0 flex-1 overflow-y-auto"
      >
        <div ref={contentRef} className="chat-column-width mx-auto flex w-full flex-col gap-5 px-5 py-5">
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

        {blocks.map(renderBlock)}

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
          {failed && outOfCredits && !includedModel && (
            <motion.div key="upgrade" {...(reduce ? TRANSIENT_INSTANT : TRANSIENT)}>
              <UpgradePrompt />
            </motion.div>
          )}
          {failed && includedModel && includedWeeklyAllowanceExhausted && (
            <motion.div key="included-weekly-usage" {...(reduce ? TRANSIENT_INSTANT : TRANSIENT)}>
              <div className="rounded-xl border border-warning/30 bg-warning/10 px-4 py-3 text-sm text-ink">
                Your included weekly usage is used up. It resets on Monday.
              </div>
            </motion.div>
          )}
          {verificationIncomplete && (
            <motion.div
              key="verification-incomplete"
              {...(reduce ? TRANSIENT_INSTANT : TRANSIENT)}
              className={cn(WARNING_BANNER, "flex items-start gap-2")}
            >
              <div className="min-w-0 flex-1">
                <span className="font-medium text-warning">Verification incomplete.</span>{" "}
                {humanizeRunFailure(verificationIncomplete.outcome)}
              </div>
              <DismissButton
                muted
                onClick={() => dismissFailedRun(verificationIncomplete.id)}
              />
            </motion.div>
          )}
          {failed && !outOfCredits && !interrupted && !verificationIncomplete && (
            <motion.div
              key="failed"
              {...(reduce ? TRANSIENT_INSTANT : TRANSIENT)}
              className={cn(DANGER_BANNER, "flex items-start gap-2")}
            >
              <div className="min-w-0 flex-1">
                <span className="font-medium">Run failed.</span>{" "}
                {humanizeRunFailure(failed.outcome)}
              </div>
              <DismissButton onClick={() => dismissFailedRun(failed.id)} />
            </motion.div>
          )}
          {interrupted && (
            <motion.div
              key="interrupted"
              {...(reduce ? TRANSIENT_INSTANT : TRANSIENT)}
              className={cn(STOPPED_BANNER, "flex items-start gap-2")}
            >
              <div className="min-w-0 flex-1">
                <span className="font-medium text-ink-secondary">Run interrupted.</span>{" "}
                {humanizeRunFailure(interrupted.outcome)}
              </div>
              <DismissButton muted onClick={() => dismissFailedRun(interrupted.id)} />
            </motion.div>
          )}
          {stopped && (
            <motion.div
              key="stopped"
              {...(reduce ? TRANSIENT_INSTANT : TRANSIENT)}
              className={cn(STOPPED_BANNER, "flex items-start gap-2")}
            >
              <div className="min-w-0 flex-1">
                <span className="font-medium text-ink-secondary">Run stopped before finishing.</span>{" "}
                Completed work is preserved; in-flight actions were cancelled.
              </div>
              <DismissButton muted onClick={() => dismissFailedRun(stopped.id)} />
            </motion.div>
          )}
          {error && (
            <motion.div
              key="error"
              {...(reduce ? TRANSIENT_INSTANT : TRANSIENT)}
              className={cn(DANGER_BANNER, "flex items-start gap-2")}
            >
              <div className="min-w-0 flex-1">{humanizeError(error)}</div>
              <DismissButton onClick={dismissError} />
            </motion.div>
          )}
        </AnimatePresence>
        </div>
      </div>

      {/* Keep this outside the scrollable transcript. Mounting or unmounting a
          control in that flow changes scrollHeight, which can clamp scrollTop
          and look like a fresh upward scroll. */}
      <AnimatePresence>
        {!atBottom && visible.length > 0 && (
          <motion.button
            onClick={scrollToBottom}
            initial={reduce ? false : { opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={reduce ? { opacity: 0, transition: { duration: 0 } } : { opacity: 0, y: 8 }}
            transition={{ duration: DUR.fast, ease: EASE.out }}
            className="absolute bottom-4 left-1/2 z-10 flex w-fit -translate-x-1/2 items-center gap-1.5 rounded-full bg-bg-elevated px-3 py-1.5 text-xs font-medium text-ink-secondary shadow-lg ring-1 ring-border-subtle transition-colors hover:text-ink"
          >
            <ArrowDown className="size-3.5" /> Jump to latest
          </motion.button>
        )}
      </AnimatePresence>

      {/* `/btw` side-question overlay — a fixed modal above the transcript.
          Renders only while a side question is open; the active run keeps
          streaming behind the translucent backdrop. */}
      <SideQuestionCard />
    </div>
  );
}
