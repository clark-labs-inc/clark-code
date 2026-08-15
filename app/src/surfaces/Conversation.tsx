import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { ArrowDown, X } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { effectiveApprovalPolicy } from "../store/sessionStore.runtime";
import { approvalPolicyForSpecialist, wouldAutoApprove } from "../lib/permissions";
import {
  currentActivity,
  isAwaitingAssistantReply,
  isThinkingOnlyMessage,
  shouldShowPending,
} from "../lib/activity";
import {
  humanizeError,
  humanizeRunFailure,
  isQuietRetryableRunFailure,
} from "../lib/errors";
import { cn } from "../lib/cn";
import {
  commitChatRowKeys,
  createChatRowMotionState,
  enteringChatRowKeys,
  EXPAND,
  EXPAND_REDUCED,
  FADE,
  accessibleMotion,
} from "../lib/motion";
import {
  conversationScrollTarget,
  isConversationAtBottom,
  isConversationScrollUp,
  shouldFollowConversation,
  type ConversationScrollState,
} from "../lib/conversationScroll";
import { conversationBlockWindow, type ConversationBlock } from "../lib/conversationBlocks";
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
import { SpecialistConversationPresentationCard } from "./specialists/SpecialistConversationShowcase";
import { ReplySkeleton, STREAMING_REPLY_RESERVE_LINES } from "./StreamingReply";
import type { Artifact, ToolCall } from "../core-bridge/types";
import { effectiveModelSettings, isIncludedCodingModel } from "../lib/localAgent";
import { specialistPresentationFromPayload } from "../lib/specialistPresentation";
import { hasTerminalProviderIncident } from "../lib/providerIncidentPresentation";

/** The only live-work indicator in the transcript. CSS owns both its pulse and
 * reduced-motion fallback so the visual policy cannot diverge from skeletons. */
const ActivityDots = memo(function ActivityDots() {
  return (
    <span className="activity-dots flex shrink-0 items-center gap-[3px]" aria-hidden>
      <span className="size-1.5 rounded-full bg-accent" />
      <span className="size-1.5 rounded-full bg-accent" />
      <span className="size-1.5 rounded-full bg-accent" />
    </span>
  );
});

/** "Working now" — dots + label, plus a skeleton preview before the first
 *  tokens of the reply arrive. Hidden while a tool line shows its own spinner. */
function Pending({
  label,
  detail,
  skeleton,
}: {
  label: string;
  detail?: string;
  skeleton: boolean;
}) {
  const activity = (
    <div className="flex items-center gap-2.5 text-sm text-ink-muted">
      <ActivityDots />
      <span className="truncate">
        {label || "Thinking…"}
        {detail && <span className="ml-1.5 font-mono text-xs text-ink-faint">{detail}</span>}
      </span>
    </div>
  );

  if (skeleton) {
    return (
      <div className="reply-stream-reserve text-base" data-qa="reply-skeleton-reserve">
        <div className="reply-stream-line">{activity}</div>
        <ReplySkeleton lines={STREAMING_REPLY_RESERVE_LINES - 1} startIndex={1} />
      </div>
    );
  }

  return (
    activity
  );
}

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
  onOpenArtifact,
}: {
  onOpenArtifact?: (artifact: Artifact) => void;
}) {
  const reduce = useReducedMotion();
  const snapshot = useSessionStore((s) => s.snapshot);
  const session = useSessionStore((s) => s.session);
  const approvalPolicy = useSessionStore((s) => s.approvalPolicy);
  const approvalPolicies = useSessionStore((s) => s.approvalPolicies);
  const specialistKind = useSessionStore((s) => s.conversations.find(
    (conversation) => conversation.id === s.session?.id,
  )?.specialist?.kind);
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
  const scrollFrameRef = useRef<number | null>(null);
  const rowMotionRef = useRef(createChatRowMotionState());
  const pinnedScrollActive = useRef(false);
  const scrollingToBottom = useRef(false);
  const upwardWheel = useRef(false);
  const [showAll, setShowAll] = useState(false);
  const activity = currentActivity(snapshot);
  // Collapse history again when switching conversations.
  useEffect(() => setShowAll(false), [sessionId]);
  // Pin to the bottom only when the user is already there — never yank them up
  // while they're reading scrollback. A small rAF follower absorbs uneven text
  // batches and tool-card height changes into continuous viewport movement.
  const stuck = useRef(true);
  const [atBottom, setAtBottom] = useState(true);
  const cancelPinnedScroll = useCallback(() => {
    if (scrollFrameRef.current !== null) {
      cancelAnimationFrame(scrollFrameRef.current);
      scrollFrameRef.current = null;
    }
    pinnedScrollActive.current = false;
  }, []);
  const schedulePinnedScroll = useCallback(() => {
    if (!sessionId || !stuck.current || scrollFrameRef.current !== null) return;
    pinnedScrollActive.current = true;

    const scroll = () => {
      scrollFrameRef.current = null;
      if (!stuck.current) {
        pinnedScrollActive.current = false;
        return;
      }
      const el = scrollRef.current;
      if (!el) {
        pinnedScrollActive.current = false;
        return;
      }

      const target = Math.max(0, el.scrollHeight - el.clientHeight);
      if (Math.abs(el.scrollTop - target) < 0.5) {
        pinnedScrollActive.current = false;
        return;
      }
      // One frame is enough to coalesce layout changes. Chasing the target with
      // recursive easing keeps WebKit's scrolling/compositing tree active for
      // many frames after every streamed update and makes input feel sticky.
      el.scrollTop = target;
      lastScrollTop.current = el.scrollTop;
      scrollByConversation.set(sessionId, { scrollTop: el.scrollTop, atBottom: true });
      pinnedScrollActive.current = false;
    };

    if (typeof requestAnimationFrame === "undefined") {
      scroll();
      return;
    }
    scrollFrameRef.current = requestAnimationFrame(scroll);
  }, [sessionId]);
  useEffect(() => () => cancelPinnedScroll(), [cancelPinnedScroll]);
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
      scrollingToBottom.current || pinnedScrollActive.current,
      userScrolledUp,
    );
    lastScrollTop.current = el.scrollTop;
    if (userScrolledUp) cancelPinnedScroll();
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
      cancelPinnedScroll();
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
  // Mode flips (e.g. Shift+Tab to "Full access") auto-grant a pending request:
  // unmount this request's AnimatePresence child so its exit animation runs on
  // the full card, instead of PermissionGate tearing its own content out mid-
  // frame. Requests the policy already grants before they ever reach the gate
  // still never mount here.
  const permissionAutoGranted = pending_permission
    ? wouldAutoApprove(
        approvalPolicyForSpecialist(
          effectiveApprovalPolicy(approvalPolicy, approvalPolicies, sessionId),
          specialistKind,
        ),
        pending_permission,
      )
    : false;
  const showPermissionGate = !!pending_permission && !permissionAutoGranted;
  const blockWindow = useMemo(
    () => conversationBlockWindow(timeline, goal, showAll, TIMELINE_WINDOW),
    [goal, showAll, timeline],
  );
  const { blocks, rowKeys, windowed } = blockWindow;
  const enteringRows = enteringChatRowKeys(rowMotionRef.current, sessionId, rowKeys);
  useLayoutEffect(() => {
    commitChatRowKeys(rowMotionRef.current, sessionId, rowKeys);
  }, [rowKeys, sessionId]);

  // Restore after React has committed the target transcript but before paint,
  // avoiding a frame at the previous conversation's unrelated scrollTop.
  useLayoutEffect(() => {
    cancelPinnedScroll();
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
  }, [cancelPinnedScroll, sessionId]);

  useEffect(() => {
    schedulePinnedScroll();
  }, [schedulePinnedScroll, sessionId, timeline, toolCalls]);

  // Timeline rows, images, and animated pending/permission banners can change
  // height after their snapshot render. Follow the actual content box while
  // pinned so "latest" remains truly visible rather than a few pixels below
  // the viewport after an enter animation settles.
  useEffect(() => {
    const el = scrollRef.current;
    const content = contentRef.current;
    if (!el || !content || !sessionId || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(schedulePinnedScroll);
    observer.observe(content);
    return () => observer.disconnect();
  }, [schedulePinnedScroll, sessionId]);

  if (!session) return null;

  const visible = timeline;
  const transientMotion = reduce ? EXPAND_REDUCED : EXPAND;
  // Long transcripts: render only the recent window. A 400-item DOM makes every
  // style/layout pass (and each streamed frame) pay for history the user isn't
  // reading — the dominant cost on slower machines. "Show earlier" reveals all.
  const awaitingReply = isAwaitingAssistantReply(visible);
  // Tool rows and actively streaming unphased responses own their live state;
  // completed commentary keeps this row visible until the run advances or ends.
  const showPending = shouldShowPending(snapshot);
  const toolReplyReserve = awaitingReply && activity.busy && !showPending;
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
  const interrupted = failed?.outcome?.failure_kind === "runtime_interrupted" ? failed : undefined;
  const verificationIncomplete =
    failed?.outcome?.failure_kind === "verification_incomplete" ? failed : undefined;
  const quietRetryableFailure = isQuietRetryableRunFailure(failed?.outcome) ? failed : undefined;
  const terminalProviderFailure = hasTerminalProviderIncident(
    timeline,
    providerIncidents,
    latestRun?.id,
  );

  const renderBlock = (block: ConversationBlock) => {
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
      const streaming =
        activity.busy &&
        block.timelineIndex === visible.length - 1 &&
        item.role === "agent";
      // The canonical live state is the pending row below. Do not mount an
      // empty-looking second Thinking row until it becomes durable history or
      // shares the message with actual answer text.
      if (streaming && isThinkingOnlyMessage(item)) return null;
      return (
        <Message
          key={block.key}
          role={item.role}
          blocks={item.blocks}
          phase={item.phase}
          timelineIndex={block.timelineIndex}
          animateEntry={enteringRows.has(block.key)}
          streaming={streaming}
        />
      );
    }
    if (item.item === "specialist_presentation") {
      const presentation = specialistPresentationFromPayload(item.presentation);
      return presentation ? (
        <SpecialistConversationPresentationCard
          key={block.key}
          presentation={presentation}
          variant="conversation"
        />
      ) : null;
    }
    if (item.item === "artifact") {
      const artifact = artifacts.find((candidate) => candidate.id === item.id);
      return artifact ? (
        <div
          id={`artifact-${artifact.id}`}
          key={block.key}
          tabIndex={-1}
          className="group/artifact relative outline-none"
        >
          <ArtifactCard
            artifact={artifact}
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
          modelRouteLabel={session.provider === "local" ? "the agent's cloud model gateway" : "the selected model provider"}
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
        role="log"
        aria-label="Conversation"
        className="min-h-0 flex-1 overflow-y-auto"
      >
        <div ref={contentRef} className="conversation-column-width mx-auto flex w-full flex-col gap-3 px-5 py-5">
        {visible.length === 0 && !showPending && (
          <p className="py-10 text-center text-sm text-ink-faint">
            Ask the agent anything — file work, web research, and computer use show up here as it works.
          </p>
        )}

        {windowed && (
          <button
            onClick={() => setShowAll(true)}
            className="mx-auto rounded-full border border-border-subtle bg-bg-elevated px-3.5 py-1.5 text-xs font-medium text-ink-muted transition hover:bg-bg-hover hover:text-ink-secondary"
          >
            Show earlier history
          </button>
        )}

        {blocks.map(renderBlock)}

        {toolReplyReserve && (
          <div data-qa="reply-tool-reserve">
            <ReplySkeleton lines={1} startIndex={STREAMING_REPLY_RESERVE_LINES - 1} />
          </div>
        )}

        <FanOutPanel />

        {/* Default (sync) mode, not popLayout: popLayout yanks an exiting
            banner OUT of the layout flow, so a collapsing Pending briefly
            floats over the content below it. In-flow exit collapses height in
            place — no overlap. */}
        <AnimatePresence initial={false}>
          {showPending && (
            <m.div key="pending" {...transientMotion}>
              <Pending label={activity.label} detail={activity.detail} skeleton={awaitingReply} />
            </m.div>
          )}
          {pending_permission && showPermissionGate && (
            <m.div key="permission" {...transientMotion}>
              <PermissionGate req={pending_permission} />
            </m.div>
          )}
          {outOfCredits && failed && (
            <m.div key="upgrade" {...transientMotion}>
              <UpgradePrompt
                error={failed.outcome?.error}
                includedModel={includedModel}
              />
            </m.div>
          )}
          {verificationIncomplete && (
            <m.div
              key="verification-incomplete"
              {...transientMotion}
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
            </m.div>
          )}
          {failed && !outOfCredits && !interrupted && !verificationIncomplete && !quietRetryableFailure && !terminalProviderFailure && (
            <m.div
              key="failed"
              {...transientMotion}
              className={cn(DANGER_BANNER, "flex items-start gap-2")}
            >
              <div className="min-w-0 flex-1">
                <span className="font-medium">Run failed.</span>{" "}
                {humanizeRunFailure(failed.outcome)}
              </div>
              <DismissButton onClick={() => dismissFailedRun(failed.id)} />
            </m.div>
          )}
          {quietRetryableFailure && !terminalProviderFailure && (
            <m.div
              key="quiet-retryable-failure"
              {...transientMotion}
              className={cn(STOPPED_BANNER, "flex items-start gap-2")}
            >
              <div className="min-w-0 flex-1">
                <span className="font-medium text-ink-secondary">Taking a little longer.</span>{" "}
                {humanizeRunFailure(quietRetryableFailure.outcome)}
              </div>
              <DismissButton muted onClick={() => dismissFailedRun(quietRetryableFailure.id)} />
            </m.div>
          )}
          {interrupted && (
            <m.div
              key="interrupted"
              {...transientMotion}
              className={cn(STOPPED_BANNER, "flex items-start gap-2")}
            >
              <div className="min-w-0 flex-1">
                <span className="font-medium text-ink-secondary">Run interrupted.</span>{" "}
                {humanizeRunFailure(interrupted.outcome)}
              </div>
              <DismissButton muted onClick={() => dismissFailedRun(interrupted.id)} />
            </m.div>
          )}
          {stopped && (
            <m.div
              key="stopped"
              {...transientMotion}
              className={cn(STOPPED_BANNER, "flex items-start gap-2")}
            >
              <div className="min-w-0 flex-1">
                <span className="font-medium text-ink-secondary">Run stopped before finishing.</span>{" "}
                Completed work is preserved; in-flight actions were cancelled.
              </div>
              <DismissButton muted onClick={() => dismissFailedRun(stopped.id)} />
            </m.div>
          )}
          {error && !terminalProviderFailure && (
            <m.div
              key="error"
              {...transientMotion}
              className={cn(DANGER_BANNER, "flex items-start gap-2")}
            >
              <div className="min-w-0 flex-1">{humanizeError(error)}</div>
              <DismissButton onClick={dismissError} />
            </m.div>
          )}
        </AnimatePresence>
        </div>
      </div>

      {/* Keep this outside the scrollable transcript. Mounting or unmounting a
          control in that flow changes scrollHeight, which can clamp scrollTop
          and look like a fresh upward scroll. */}
      <AnimatePresence>
        {!atBottom && visible.length > 0 && (
          <m.button
            onClick={scrollToBottom}
            {...accessibleMotion(FADE, reduce)}
            className="popover-surface absolute bottom-4 left-1/2 z-10 flex w-fit -translate-x-1/2 items-center gap-1.5 rounded-full bg-bg-elevated px-3 py-1.5 text-xs font-medium text-ink-secondary shadow-lg ring-1 ring-border-subtle transition-colors hover:text-ink"
          >
            <ArrowDown className="size-3.5" /> Jump to latest
          </m.button>
        )}
      </AnimatePresence>

      {/* `/btw` side-question overlay — a fixed modal above the transcript.
          Renders only while a side question is open; the active run keeps
          streaming behind the translucent backdrop. */}
      <SideQuestionCard />
    </div>
  );
}
