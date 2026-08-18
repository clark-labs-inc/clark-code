import type { GoalState, TimelineItem } from "../core-bridge/types";

export type ConversationBaseBlock =
  | { kind: "item"; item: TimelineItem; timelineIndex: number; key: string }
  | { kind: "work"; ids: string[]; run?: string; key: string };
export type ConversationBlock =
  | ConversationBaseBlock
  | { kind: "goal_work"; blocks: ConversationBaseBlock[]; key: string };

function group(
  timeline: TimelineItem[],
  goal: GoalState | undefined,
  timelineOffset: number,
): ConversationBlock[] {
  const base: ConversationBaseBlock[] = [];
  timeline.forEach((item, localIndex) => {
    const timelineIndex = timelineOffset + localIndex;
    if (item.item === "tool_call") {
      const last = base[base.length - 1];
      if (last && last.kind === "work" && last.run === item.run) last.ids.push(item.id);
      else base.push({ kind: "work", ids: [item.id], run: item.run, key: `w${timelineIndex}` });
    } else {
      base.push({ kind: "item", item, timelineIndex, key: `i${timelineIndex}` });
    }
  });

  if (!goal?.run) return base;
  const blocks: ConversationBlock[] = [];
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

export interface ConversationBlockWindow {
  blocks: ConversationBlock[];
  rowKeys: string[];
  windowed: boolean;
  start: number;
  end: number;
  hasEarlier: boolean;
  hasLater: boolean;
}

/**
 * Group one fixed-size raw timeline page. Paging by raw items (rather than by
 * grouped blocks) is load-bearing: a single work block can contain thousands
 * of contiguous tool calls, so a block-count limit alone does not bound DOM or
 * diff-summary work. `endExclusive === null` follows the live tail.
 */
export function conversationBlockWindow(
  timeline: TimelineItem[],
  goal: GoalState | undefined,
  endExclusive: number | null,
  limit: number,
): ConversationBlockWindow {
  const pageSize = Math.max(1, Math.floor(limit));
  const end = endExclusive === null
    ? timeline.length
    : Math.max(0, Math.min(timeline.length, Math.floor(endExclusive)));
  const start = Math.max(0, end - pageSize);
  const blocks = group(timeline.slice(start, end), goal, start);
  const rowKeys = blocks.flatMap((block) =>
    block.kind === "goal_work" ? block.blocks.map((child) => child.key) : [block.key]
  );
  const hasEarlier = start > 0;
  const hasLater = end < timeline.length;
  return {
    blocks,
    rowKeys,
    windowed: hasEarlier || hasLater,
    start,
    end,
    hasEarlier,
    hasLater,
  };
}
