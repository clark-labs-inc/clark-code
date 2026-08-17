import type { TimelineItem } from "../core-bridge/types";
import { parseScopedSpecPrompt } from "./specDocuments";

export interface SpecSelectionTurn {
  question: string;
  reply: string;
  runId: string;
}

export interface SpecSelectionConversation {
  key: string;
  label: string;
  selection: string;
  turns: SpecSelectionTurn[];
}

function messageText(item: Extract<TimelineItem, { item: "message" }>): string {
  return item.blocks
    .filter((block) => block.type === "text")
    .map((block) => block.text)
    .join("\n")
    .trim();
}

export function specSelectionKey(label: string): string {
  return label.trim().toLocaleLowerCase().replace(/\s+/g, " ").slice(0, 200);
}

/** Rebuild section conversations from the canonical transcript so closing,
 * switching sections, or reopening a saved Spec never mixes local chat state. */
export function specSelectionConversations(
  timeline: readonly TimelineItem[],
): Record<string, SpecSelectionConversation> {
  const conversations: Record<string, SpecSelectionConversation> = {};
  const turnByRun = new Map<string, SpecSelectionTurn>();

  for (const item of timeline) {
    if (item.item !== "message") continue;
    if (item.role === "user") {
      const scoped = parseScopedSpecPrompt(messageText(item));
      if (!scoped) continue;
      const key = specSelectionKey(scoped.section);
      const conversation = conversations[key] ?? {
        key,
        label: scoped.section,
        selection: scoped.selection,
        turns: [],
      };
      const turn = { question: scoped.comment, reply: "", runId: item.run };
      conversation.label = scoped.section;
      conversation.selection = scoped.selection;
      conversation.turns.push(turn);
      conversations[key] = conversation;
      turnByRun.set(item.run, turn);
      continue;
    }

    const turn = turnByRun.get(item.run);
    if (!turn) continue;
    const reply = messageText(item);
    if (reply) turn.reply = reply;
  }

  return conversations;
}
