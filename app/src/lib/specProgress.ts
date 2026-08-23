import { isAwaitingAssistantReply, isThinkingOnlyMessage, lastProgressLine } from "./activity";
import { summarizeEdits } from "./diff";
import { TOOL_KIND_LABEL } from "./toolLabels";
import type { Snapshot, TimelineItem, ToolCall } from "../core-bridge/types";

/** Strip provider/tool protocol prefixes from copy shown to a person. Returns
 * undefined when nothing readable is left, so a single fallback owner (the
 * ladder in `specLiveStatus`) decides what to say instead. */
export function specProgressTitle(call: Pick<ToolCall, "title">): string | undefined {
  return call.title.replace(/^[a-z][a-z0-9_]*:\s*/i, "").trim() || undefined;
}

/** Tool calls for only the latest user turn, in durable timeline order. This
 * keeps a reopened Spec from presenting old work as current live progress. */
export function currentSpecToolCalls(snapshot: Snapshot): ToolCall[] {
  let latestUserIndex = -1;
  for (let index = snapshot.timeline.length - 1; index >= 0; index -= 1) {
    const item = snapshot.timeline[index];
    if (item.item === "message" && item.role === "user") {
      latestUserIndex = index;
      break;
    }
  }

  const seen = new Set<string>();
  const calls: ToolCall[] = [];
  for (const item of snapshot.timeline.slice(latestUserIndex + 1)) {
    if (item.item !== "tool_call" || seen.has(item.id)) continue;
    const call = snapshot.tool_calls[item.id];
    if (!call) continue;
    seen.add(item.id);
    calls.push(call);
  }
  return calls;
}

/** Which rung of the ladder produced a label. Asserted by tests so the ordering
 *  is pinned, and used to pick a coarse screen-reader sentence that doesn't
 *  re-announce on every streamed token. */
export type SpecLabelSource =
  | "tool_progress" | "tool_stream" | "tool_title" | "checklist"
  | "drafting" | "commentary" | "thinking" | "last_receipt" | "starting" | "unknown";

export interface SpecLiveStatus {
  label: string;
  /** Target of the active call — a path — shown beside the label. */
  detail?: string;
  source: SpecLabelSource;
}

/** A streamed shell line can be arbitrarily long; a label cannot. */
const MAX_LABEL = 120;

function clamp(text: string): string {
  const trimmed = text.trim();
  if (trimmed.length <= MAX_LABEL) return trimmed;
  let cut = trimmed.slice(0, MAX_LABEL);
  // Streamed output can carry emoji; a UTF-16 slice can land mid-pair and a
  // lone high surrogate renders as a replacement character.
  const last = cut.charCodeAt(cut.length - 1);
  if (last >= 0xd800 && last <= 0xdbff) cut = cut.slice(0, -1);
  return `${cut}…`;
}

function hasVisibleText(item: TimelineItem): boolean {
  return item.item === "message"
    && item.blocks.some((block) => block.type === "text" && block.text.trim().length > 0);
}

/** What is happening in a Spec run right now, in the most specific terms the
 *  snapshot supports. Every rung above `unknown` is real evidence; a bare
 *  "Working…" means the snapshot genuinely carried none. */
export function specLiveStatus(
  snapshot: Snapshot,
  calls: readonly ToolCall[],
): SpecLiveStatus {
  const active = calls.find((call) => call.status === "in_progress");
  if (active) {
    const detail = active.locations?.[0]?.path;
    const reported = active.progress?.latest_activity?.trim();
    if (reported) return { label: clamp(reported), detail, source: "tool_progress" };
    const streamed = lastProgressLine(active);
    if (streamed) return { label: clamp(streamed), detail, source: "tool_stream" };
    const title = specProgressTitle(active);
    if (title) return { label: clamp(title), detail, source: "tool_title" };
  }

  const step = snapshot.execution_checklist?.steps.find((s) => s.status === "in_progress");
  if (step) return { label: clamp(step.title), source: "checklist" };

  // What is happening now beats what just happened, so a reply already streaming
  // outranks the receipt for the tool call that preceded it.
  const last = snapshot.timeline[snapshot.timeline.length - 1];
  if (last?.item === "message" && last.role === "agent" && hasVisibleText(last)) {
    return last.phase === "commentary"
      ? { label: "Explaining the change…", source: "commentary" }
      : { label: "Writing the spec…", source: "drafting" };
  }

  if (!last || (last.item === "message" && last.role === "user") || isThinkingOnlyMessage(last)) {
    return { label: "Thinking it through…", source: "thinking" };
  }

  if (last.item === "tool_call") {
    const finished = calls[calls.length - 1];
    if (finished) {
      const what = specProgressTitle(finished) ?? TOOL_KIND_LABEL[finished.kind];
      return { label: clamp(`Finished ${what}`), source: "last_receipt" };
    }
  }

  if (isAwaitingAssistantReply(snapshot.timeline) && calls.length > 0) {
    return { label: "Deciding what to change next…", source: "thinking" };
  }

  if (snapshot.starting) return { label: "Getting set up…", source: "starting" };

  return { label: "Working…", source: "unknown" };
}

/** The calls a trail can show without wrapping, keeping the running call and the
 *  tail visible and eliding from the head. */
export function specTrailWindow(
  calls: readonly ToolCall[],
  max = 7,
): { hidden: number; visible: ToolCall[] } {
  if (calls.length <= max) return { hidden: 0, visible: [...calls] };

  const activeIndex = calls.findIndex((call) => call.status === "in_progress");
  // Show the tail; if the running call is older than that, give up its slot for
  // the running call so the live state is never the thing that gets elided.
  const tail = calls.slice(calls.length - max);
  if (activeIndex >= 0 && activeIndex < calls.length - max) {
    return { hidden: calls.length - max, visible: [calls[activeIndex], ...tail.slice(1)] };
  }
  return { hidden: calls.length - max, visible: tail };
}

/** What to show opposite the label. `edits` is what actually changed on disk;
 *  `steps` is the agent's own plan position. Typed rather than a bare string so a
 *  caller can tell whether a progress bar would repeat it. */
export interface SpecRunReceipt {
  text: string;
  kind: "edits" | "steps";
}

/** Returns undefined rather than inventing a number — the trail already conveys
 *  volume, and an empty slot beats a meaningless one. */
export function specRunReceipt(
  calls: readonly ToolCall[],
  steps?: { done: number; total: number },
): SpecRunReceipt | undefined {
  const edits = summarizeEdits([...calls]);
  if (edits) {
    return { text: `${edits.files} file${edits.files === 1 ? "" : "s"} changed`, kind: "edits" };
  }
  if (steps && steps.total > 0) {
    return {
      text: `Step ${Math.min(steps.done + 1, steps.total)} of ${steps.total}`,
      kind: "steps",
    };
  }
  return undefined;
}
