// Derive the single "what is happening right now" signal from a snapshot, so the
// UI can always answer: happening now? progress? Pure + tested.

import type { RunOutcome, Snapshot, ToolCall } from "../core-bridge/types";

/** Compact, typed diagnostics for the provider-local root execution receipt. */
export function executionDiagnostic(outcome?: RunOutcome): string | undefined {
  const execution = outcome?.execution;
  if (!execution) return undefined;
  const attemptLabel = execution.attempts === 1 ? "attempt" : "attempts";
  const parts = [`Root execution: ${execution.attempts} ${attemptLabel}`];
  if (execution.recoveries > 0) parts.push(`${execution.recoveries} recovered interruption${execution.recoveries === 1 ? "" : "s"}`);
  if (execution.child_executions > 0) parts.push(`${execution.completed_children}/${execution.child_executions} subagents completed`);
  if (execution.changed_paths.length > 0) parts.push(`${execution.changed_paths.length} changed path${execution.changed_paths.length === 1 ? "" : "s"}`);
  if (execution.failed_tools.length > 0) parts.push(`${execution.failed_tools.length} failed/cancelled tool${execution.failed_tools.length === 1 ? "" : "s"}`);
  return parts.join(" · ");
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

export function currentActivity(snapshot: Snapshot): Activity {
  const runs = Object.values(snapshot.runs);
  const busy = runs.some((r) => r.status === "running" || r.status === "queued");
  const failed = runs.some((r) => r.status === "failed");

  let progress: number | undefined;
  let steps: { done: number; total: number } | undefined;
  if (snapshot.plan && snapshot.plan.phases.length > 0) {
    const total = snapshot.plan.phases.length;
    const done = snapshot.plan.phases.filter((p) => p.status === "completed").length;
    steps = { done, total };
    progress = done / total;
  }

  if (!busy) {
    if (failed) return { busy: false, failed: true, label: "Run failed", progress, steps };
    return { busy: false, label: "Ready", progress, steps };
  }

  const tool = Object.values(snapshot.tool_calls).find((t) => t.status === "in_progress");
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
  const phase = snapshot.plan?.phases.find((p) => p.status === "in_progress");
  if (phase) return { busy: true, label: phase.title, progress, steps };

  return { busy: true, label: "Thinking…", progress, steps };
}

/** Whether the conversation needs a placeholder before the agent has emitted
 *  any typed response. Once an agent message exists, that message owns the live
 *  state (including native thinking blocks), so rendering another pending row
 *  would duplicate the same activity. */
export function shouldShowPending(snapshot: Snapshot): boolean {
  const activity = currentActivity(snapshot);
  if (!activity.busy) return false;
  if (Object.values(snapshot.tool_calls).some((tool) => tool.status === "in_progress")) {
    return false;
  }

  const last = snapshot.timeline[snapshot.timeline.length - 1];
  return !last || (last.item === "message" && last.role === "user");
}
