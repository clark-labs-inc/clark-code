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
}

/**
 * Group only enough recent raw timeline items to fill the visible block
 * window. Most transcripts need at most `2 * limit` items; unusually dense
 * tool/goal runs expand geometrically until the block contract is satisfied.
 * This bounds the common streamed-token path without changing what the user
 * sees or breaking stable keys/timeline indices.
 */
export function conversationBlockWindow(
  timeline: TimelineItem[],
  goal: GoalState | undefined,
  showAll: boolean,
  limit: number,
): ConversationBlockWindow {
  let start = showAll ? 0 : Math.max(0, timeline.length - limit * 2);
  let grouped = group(timeline.slice(start), goal, start);

  while (!showAll && start > 0 && grouped.length <= limit) {
    const covered = timeline.length - start;
    start = Math.max(0, timeline.length - covered * 2);
    grouped = group(timeline.slice(start), goal, start);
  }

  const windowed = !showAll && (start > 0 || grouped.length > limit);
  const blocks = windowed ? grouped.slice(-limit) : grouped;
  const rowKeys = blocks.flatMap((block) =>
    block.kind === "goal_work" ? block.blocks.map((child) => child.key) : [block.key]
  );
  return { blocks, rowKeys, windowed };
}
