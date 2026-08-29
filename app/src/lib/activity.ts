// Derive the single "what is happening right now" signal from a snapshot, so the
// UI can always answer: happening now? progress? Pure + tested.

import type { Snapshot, TimelineItem, ToolCall } from "../core-bridge/types";

/** Private reasoning is durable history, not a second live-work surface. While
 * it is the only agent output, Conversation keeps it hidden and lets the one
 * pending row own the active state. */
export function isThinkingOnlyMessage(item: TimelineItem | undefined): boolean {
  return !!item
    && item.item === "message"
    && item.role === "agent"
    && item.blocks.length > 0
    && item.blocks.every((block) => block.type === "thinking");
}

/** Whether the latest user turn still has no visible assistant prose. Plans,
 * checklists, and completed tool rows may arrive first; they must not consume
 * the reply skeleton before the reply itself has begun. */
export function isAwaitingAssistantReply(timeline: readonly TimelineItem[]): boolean {
  let latestUser = -1;
  for (let index = timeline.length - 1; index >= 0; index -= 1) {
    const item = timeline[index];
    if (item.item === "message" && item.role === "user") {
      latestUser = index;
      break;
    }
  }
  if (latestUser < 0) return timeline.length === 0;

  return !timeline.slice(latestUser + 1).some((item) =>
    item.item === "message"
    && item.role === "agent"
    && item.phase !== "commentary"
    && item.blocks.some((block) => block.type === "text" && block.text.trim().length > 0)
  );
}

/** The most recent streamed progress line of a tool call (e.g. a subagent step),
 *  stripped of bullet markers. Used to show live progress on long calls. */
export function lastProgressLine(tool: ToolCall): string | undefined {
  const text = tool.content
    .filter((b) => b.type === "text")
    .map((b) => (b.type === "text" ? b.text : ""))
    .join("");
  const lines = text.split("\n").map((l) => l.trim()).filter(Boolean);
  const last = lines[lines.length - 1];
  return last ? last.replace(/^[•\-*]\s*/, "") : undefined;
}

export interface Activity {
  /** True while the agent is actively working. */
  busy: boolean;
  /** Short human label of the current activity. */
  label: string;
  /** Optional target (path/url) for the current activity. */
  detail?: string;
  /** 0..1 plan progress, if a plan exists. */
  progress?: number;
  /** Total / done plan steps, if a plan exists. */
  steps?: { done: number; total: number };
  failed?: boolean;
}

function activeRunIds(snapshot: Snapshot): Set<string> {
  return new Set(
    Object.values(snapshot.runs)
      .filter((run) => run.status === "running" || run.status === "queued")
      .map((run) => run.id),
  );
}

/** Tool calls and checklists are durable transcript receipts. Their status can
 * be stale in restored/compacted history, so only an item explicitly owned by
 * the current run may describe what is happening now. */
function activeToolCall(snapshot: Snapshot, activeRuns: ReadonlySet<string>): ToolCall | undefined {
  for (let index = snapshot.timeline.length - 1; index >= 0; index -= 1) {
    const item = snapshot.timeline[index];
    if (item.item !== "tool_call" || !item.run || !activeRuns.has(item.run)) continue;
    const tool = snapshot.tool_calls[item.id];
    if (tool?.status === "in_progress") return tool;
  }
  return undefined;
}

function activeChecklist(snapshot: Snapshot, activeRuns: ReadonlySet<string>) {
  for (let index = snapshot.timeline.length - 1; index >= 0; index -= 1) {
    const item = snapshot.timeline[index];
    if (item.item === "execution_checklist" && item.run && activeRuns.has(item.run)) {
      return item.checklist;
    }
  }
  return undefined;
}

export function currentActivity(snapshot: Snapshot): Activity {
  const runs = Object.values(snapshot.runs);
  const activeRuns = activeRunIds(snapshot);
  // A prompt is in flight but the provider hasn't allocated a run yet
  // (attachment upload / connect handshake). That is still active work — keep
  // the working animation visible instead of a static gap after submission.
  const busy =
    snapshot.starting === true
    || runs.some((r) => r.status === "running" || r.status === "queued");
  const interrupted = runs.some((r) => r.outcome?.failure_kind === "runtime_interrupted");
  const verificationIncomplete = runs.some(
    (r) => r.outcome?.failure_kind === "verification_incomplete",
  );
  const failed = runs.some(
    (r) => r.status === "failed"
      && r.outcome?.failure_kind !== "runtime_interrupted"
      && r.outcome?.failure_kind !== "verification_incomplete",
  );

  let progress: number | undefined;
  let steps: { done: number; total: number } | undefined;
  const checklist = busy ? activeChecklist(snapshot, activeRuns) : snapshot.execution_checklist;
  if (checklist && checklist.steps.length > 0) {
    const total = checklist.steps.length;
    const done = checklist.steps.filter((step) => step.status === "completed").length;
    steps = { done, total };
    progress = done / total;
  }

  if (!busy) {
    if (interrupted) return { busy: false, label: "Run interrupted", progress, steps };
    if (verificationIncomplete) {
      return { busy: false, label: "Verification incomplete", progress, steps };
    }
    if (failed) return { busy: false, failed: true, label: "Run failed", progress, steps };
    return { busy: false, label: "Ready", progress, steps };
  }

  const tool = activeToolCall(snapshot, activeRuns);
  if (tool) {
    return {
      busy: true,
      // Prefer the latest streamed sub-step (e.g. a subagent summary) so a long
      // tool call shows live progress rather than a frozen title.
      label: lastProgressLine(tool) ?? tool.title,
      detail: tool.locations?.[0]?.path,
      progress,
      steps,
    };
  }
  const step = checklist?.steps.find((candidate) => candidate.status === "in_progress");
  if (step) return { busy: true, label: step.title, progress, steps };

  const last = snapshot.timeline[snapshot.timeline.length - 1];
  const continuing = !!last && !(last.item === "message" && last.role === "user");
  return { busy: true, label: continuing ? "Working…" : "Thinking…", progress, steps };
}

/** Whether the conversation needs a live activity row at its foot. Tool rows
 *  and an actively streaming, unphased response own their own animation. A
 *  completed commentary message does not: while the run continues, keep an
 *  explicit activity row after it so the update cannot read as a final answer. */
export function shouldShowPending(snapshot: Snapshot): boolean {
  const activity = currentActivity(snapshot);
  if (!activity.busy) return false;
  if (activeToolCall(snapshot, activeRunIds(snapshot))) {
    return false;
  }

  const last = snapshot.timeline[snapshot.timeline.length - 1];
  if (!last || (last.item === "message" && last.role === "user")) return true;
  if (last.item !== "message") return true;
  if (isThinkingOnlyMessage(last)) return true;
  return last.role === "agent" && last.phase === "commentary";
}
