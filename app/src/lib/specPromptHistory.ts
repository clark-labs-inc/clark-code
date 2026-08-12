import type { TimelineItem } from "../core-bridge/types";

const STORAGE_PREFIX = "agent-desktop.spec-prompt-history.v1.";
const MAX_STORED_PROMPTS = 8;

export const SPEC_PROMPT_HISTORY_EVENT = "agent-desktop:spec-prompt-history";

export interface SpecPromptHistoryItem {
  text: string;
  submittedAt: number;
}

function storageKey(owner: string, conversationId: string): string {
  return `${STORAGE_PREFIX}${encodeURIComponent(owner)}.${encodeURIComponent(conversationId)}`;
}

/** Strip the machine-readable context that Spec appends after the user's copy. */
export function visibleSpecPrompt(text: string): string {
  const clean = text.trim();
  const scoped = clean.match(/<scoped_comment>\s*([\s\S]*?)\s*<\/scoped_comment>/i)?.[1];
  if (scoped?.trim()) return scoped.trim();
  const contextStart = clean.indexOf("\n\nContinue the feature-specification workflow for the current SPEC.md.");
  return (contextStart >= 0 ? clean.slice(0, contextStart) : clean).trim();
}

export function loadSpecPromptHistory(
  owner: string,
  conversationId: string | null,
): SpecPromptHistoryItem[] {
  if (!conversationId) return [];
  try {
    const parsed = JSON.parse(localStorage.getItem(storageKey(owner, conversationId)) ?? "[]");
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((item): item is SpecPromptHistoryItem => (
      typeof item?.text === "string"
      && item.text.trim().length > 0
      && typeof item.submittedAt === "number"
    )).slice(-MAX_STORED_PROMPTS);
  } catch {
    return [];
  }
}

export function recordSpecPrompt(
  owner: string,
  conversationId: string | null,
  text: string,
  submittedAt = Date.now(),
): void {
  if (!conversationId) return;
  const visible = visibleSpecPrompt(text);
  if (!visible) return;
  try {
    const current = loadSpecPromptHistory(owner, conversationId);
    const previous = current.at(-1);
    const next = previous?.text === visible
      ? [...current.slice(0, -1), { text: visible, submittedAt }]
      : [...current, { text: visible, submittedAt }];
    localStorage.setItem(
      storageKey(owner, conversationId),
      JSON.stringify(next.slice(-MAX_STORED_PROMPTS)),
    );
    window.dispatchEvent(new CustomEvent(SPEC_PROMPT_HISTORY_EVENT, {
      detail: { conversationId },
    }));
  } catch {
    // Prompt history is a convenience view; storage failure must never block a turn.
  }
}

export function recentSpecPrompts(
  stored: readonly SpecPromptHistoryItem[],
  timeline: readonly TimelineItem[],
  limit = 5,
): SpecPromptHistoryItem[] {
  const fromTimeline = timeline.flatMap((item, index): SpecPromptHistoryItem[] => {
    if (item.item !== "message" || item.role !== "user") return [];
    const text = visibleSpecPrompt(item.blocks
      .filter((block) => block.type === "text")
      .map((block) => block.text)
      .join("\n"));
    return text ? [{ text, submittedAt: index }] : [];
  });
  const merged: SpecPromptHistoryItem[] = [];
  for (const item of [...fromTimeline, ...stored]) {
    const duplicate = merged.findIndex((candidate) => candidate.text === item.text);
    if (duplicate >= 0) merged.splice(duplicate, 1);
    merged.push(item);
  }
  return merged.slice(-Math.max(0, limit));
}
