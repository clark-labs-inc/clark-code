import { memo, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { ArrowDown } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { currentActivity } from "../lib/activity";
import { Message } from "./Message";
import { WorkBlock } from "./work/WorkBlock";
import { ArtifactCard } from "./work/ArtifactCard";
import { PermissionGate } from "./PermissionGate";
import { UpgradePrompt } from "./UpgradePrompt";
import { UndoBar } from "./UndoBar";
import type { TimelineItem, ToolCall } from "../core-bridge/types";

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
  | { kind: "item"; item: TimelineItem; key: string }
  | { kind: "work"; ids: string[]; key: string };

function group(timeline: TimelineItem[]): Block[] {
  const blocks: Block[] = [];
  timeline.forEach((item, i) => {
    if (item.item === "tool_call") {
      const last = blocks[blocks.length - 1];
      if (last && last.kind === "work") last.ids.push(item.id);
      else blocks.push({ kind: "work", ids: [item.id], key: `w${i}` });
    } else {
      blocks.push({ kind: "item", item, key: `i${i}` });
    }
  });
  return blocks;
}

/** Common motion props for transient elements at the foot of the conversation. */
const TRANSIENT = {
  initial: { opacity: 0, y: 4 },
  animate: { opacity: 1, y: 0 },
  exit: { opacity: 0, transition: { duration: 0.15 } },
  transition: { duration: 0.2, ease: [0.4, 0, 0.2, 1] as const },
};

const DANGER_BANNER =
  "rounded-lg border border-danger/40 bg-danger/8 px-3.5 py-2.5 text-sm text-danger";

export function Conversation() {
  // While peeking at another conversation mid-run, render its restored
  // transcript; the live snapshot keeps streaming (and saving) underneath.
  const snapshot = useSessionStore((s) => (s.peek ? s.peek.snapshot : s.snapshot));
  const peeking = useSessionStore((s) => s.peek !== null);
  const session = useSessionStore((s) => s.session);
  const liveTitle = useSessionStore((s) =>
    s.peek && s.session ? s.conversations.find((c) => c.id === s.session!.id)?.title : undefined,
  );
  const openConversation = useSessionStore((s) => s.openConversation);
  const error = useSessionStore((s) => s.error);
  const scrollRef = useRef<HTMLDivElement>(null);
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
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
  };

  const { timeline, tool_calls: toolCalls, artifacts, runs, pending_permission } = snapshot;

  useEffect(() => {
    const el = scrollRef.current;
    if (el && stuck.current) el.scrollTop = el.scrollHeight;
  }, [timeline, toolCalls]);

  if (!session) return null;

  const visible = timeline.filter((t) => t.item !== "plan");
  const blocks = group(visible);
  const lastBlockKey = blocks[blocks.length - 1]?.key;

  const activity = currentActivity(snapshot);
  const toolActive = Object.values(toolCalls).some((t) => t.status === "in_progress");
  const last = visible[visible.length - 1];
  const awaitingReply = !last || (last.item === "message" && last.role === "user");
  // Only show the "thinking" indicator while the model is producing text (after a
  // message) — not in the gap between sequential tool calls, where it would
  // otherwise flicker in and out as each tool starts.
  const lastIsMessage = !last || last.item === "message";
  const showPending = activity.busy && !toolActive && lastIsMessage;
  const failed = Object.values(runs).find((r) => r.status === "failed");
  const outOfCredits = !!failed?.outcome?.error?.includes("insufficient_credits");
  // Offer "undo" for the most recent finished run that snapshotted the tree, but
  // only if the agent actually changed files this session.
  const madeEdits = Object.values(toolCalls).some((t) => t.kind === "edit");
  const undoSha =
    !activity.busy && madeEdits
      ? [...Object.values(runs)]
          .reverse()
          .find((r) => r.status !== "running" && r.status !== "queued" && r.checkpoint)?.checkpoint
      : undefined;

  return (
    <div ref={scrollRef} onScroll={onScroll} className="flex-1 overflow-y-auto">
      <div className="mx-auto flex max-w-3xl flex-col gap-3.5 px-5 py-6">
        {peeking && (
          <div className="flex items-center gap-2.5 rounded-lg border border-border-subtle bg-bg-elevated/70 px-3.5 py-2 text-xs text-ink-muted">
            <Dots />
            <span className="min-w-0 flex-1 truncate">
              Clark is still working{liveTitle ? <> in <span className="font-medium text-ink-secondary">“{liveTitle}”</span></> : " in another chat"} — you're viewing history.
            </span>
            <button
              onClick={() => session && void openConversation(session.id)}
              className="shrink-0 rounded-md px-2 py-1 font-medium text-ink-secondary transition hover:bg-bg-hover hover:text-ink"
            >
              Return
            </button>
          </div>
        )}
        {visible.length === 0 && !showPending && (
          <p className="py-10 text-center text-sm text-ink-faint">
            Ask Clark anything — file work, web research, and computer use show up here as it works.
          </p>
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
                streaming={activity.busy && block.key === lastBlockKey && item.role === "agent"}
              />
            );
          if (item.item === "artifact") {
            const a = artifacts.find((x) => x.id === item.id);
            return a ? <ArtifactCard key={block.key} artifact={a} /> : null;
          }
          return null;
        })}

        {undoSha && <UndoBar key={undoSha} sha={undoSha} />}

        <AnimatePresence initial={false}>
          {showPending && (
            <motion.div key="pending" {...TRANSIENT}>
              <Pending label={activity.label} detail={activity.detail} skeleton={awaitingReply} />
            </motion.div>
          )}
          {pending_permission && (
            <motion.div key="permission" {...TRANSIENT}>
              <PermissionGate req={pending_permission} />
            </motion.div>
          )}
          {failed && outOfCredits && (
            <motion.div key="upgrade" {...TRANSIENT}>
              <UpgradePrompt />
            </motion.div>
          )}
          {failed && !outOfCredits && (
            <motion.div key="failed" {...TRANSIENT} className={DANGER_BANNER}>
              <span className="font-medium">Run failed.</span>{" "}
              {failed.outcome?.error ?? "The agent ended unexpectedly."}
            </motion.div>
          )}
          {error && (
            <motion.div key="error" {...TRANSIENT} className={DANGER_BANNER}>
              {error}
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      {/* Jump-to-latest: a sticky pill (stays in the scroll flow — no positioned
          ancestor) shown only when the user has scrolled up during/after a run. */}
      {!atBottom && visible.length > 0 && (
        <button
          onClick={scrollToBottom}
          className="sticky bottom-4 left-1/2 z-10 mx-auto flex w-fit -translate-x-1/2 items-center gap-1.5 rounded-full bg-bg-elevated px-3 py-1.5 text-xs font-medium text-ink-secondary shadow-lg ring-1 ring-border-subtle transition hover:text-ink"
        >
          <ArrowDown className="size-3.5" /> Jump to latest
        </button>
      )}
    </div>
  );
}
