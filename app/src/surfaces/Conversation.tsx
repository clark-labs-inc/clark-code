import { useEffect, useRef } from "react";
import { motion, useReducedMotion } from "motion/react";
import { useSessionStore } from "../store/sessionStore";
import { currentActivity } from "../lib/activity";
import { Message } from "./Message";
import { WorkLine } from "./work/WorkLine";
import { ArtifactCard } from "./work/ArtifactCard";
import { PermissionGate } from "./PermissionGate";
import type { TimelineItem } from "../core-bridge/types";

/** Natural, in-chat "working now" line — replaces the old bottom status bar. */
function WorkingIndicator() {
  const snapshot = useSessionStore((s) => s.snapshot);
  const reduce = useReducedMotion();
  const activity = currentActivity(snapshot);
  if (!activity.busy) return null;
  return (
    <motion.div
      initial={reduce ? false : { opacity: 0 }}
      animate={{ opacity: 1 }}
      className="flex items-center gap-2.5 pl-[1.875rem] text-sm text-ink-muted"
    >
      <span className="flex items-center gap-[3px]" aria-hidden>
        {[0, 1, 2].map((i) => (
          <motion.span
            key={i}
            className="size-1.5 rounded-full bg-accent"
            animate={reduce ? undefined : { opacity: [0.3, 1, 0.3] }}
            transition={{ duration: 1.1, repeat: Infinity, delay: i * 0.18 }}
          />
        ))}
      </span>
      <span className="truncate">
        {activity.label}
        {activity.detail && (
          <span className="ml-1.5 font-mono text-xs text-ink-faint">{activity.detail}</span>
        )}
      </span>
    </motion.div>
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

function FailedBanner() {
  const runs = useSessionStore((s) => s.snapshot.runs);
  const failed = Object.values(runs).find((r) => r.status === "failed");
  if (!failed) return null;
  return (
    <div className="rounded-lg border border-danger/40 bg-danger/8 px-3.5 py-2.5 text-sm text-danger">
      <span className="font-medium">Run failed.</span>{" "}
      {failed.outcome?.error ?? "The agent ended unexpectedly."}
    </div>
  );
}

export function Conversation() {
  const timeline = useSessionStore((s) => s.snapshot.timeline);
  const toolCalls = useSessionStore((s) => s.snapshot.tool_calls);
  const artifacts = useSessionStore((s) => s.snapshot.artifacts);
  const session = useSessionStore((s) => s.session);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
  }, [timeline, toolCalls]);

  if (!session) return null;

  // Work unfolds naturally top-to-bottom — narration, work lines, and artifacts —
  // with no separate plan/phase stepper. Dropping plan items also lets adjacent
  // tool-call work merge into one dense block.
  const visible = timeline.filter((t) => t.item !== "plan");
  const blocks = group(visible);

  return (
    <div ref={scrollRef} className="flex-1 overflow-y-auto">
      <div className="mx-auto flex max-w-3xl flex-col gap-3 px-5 py-6">
        {visible.length === 0 && (
          <p className="py-10 text-center text-sm text-ink-faint">
            Ask Clark anything — file work, web research, and computer use show up here as it works.
          </p>
        )}

        {blocks.map((block) => {
          if (block.kind === "work") {
            return (
              <div
                key={block.key}
                className="divide-y divide-border-subtle overflow-hidden rounded-lg border border-border-subtle bg-bg-elevated/30"
              >
                {block.ids.map((id) => {
                  const call = toolCalls[id];
                  return call ? (
                    <WorkLine key={id} call={call} active={call.status === "in_progress"} />
                  ) : null;
                })}
              </div>
            );
          }
          const { item } = block;
          if (item.item === "message")
            return <Message key={block.key} role={item.role} blocks={item.blocks} />;
          if (item.item === "artifact") {
            const a = artifacts.find((x) => x.id === item.id);
            return a ? <ArtifactCard key={block.key} artifact={a} /> : null;
          }
          return null;
        })}

        <WorkingIndicator />
        <PermissionGate />
        <FailedBanner />
      </div>
    </div>
  );
}
