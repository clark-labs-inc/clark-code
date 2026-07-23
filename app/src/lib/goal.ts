import type { GoalState, TimelineItem } from "../core-bridge/types";

export function formatGoalDuration(seconds: number): string {
  const whole = Math.max(0, Math.floor(seconds));
  if (whole < 60) return `${whole}s`;
  const hours = Math.floor(whole / 3600);
  const minutes = Math.floor((whole % 3600) / 60);
  const remainder = whole % 60;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m ${remainder}s`;
}

export function goalElapsedSeconds(
  goal: GoalState,
  now = Date.now(),
  working = goal.status === "active",
): number {
  if (!working) return goal.time_used_seconds;
  const live = Math.max(0, Math.floor((now - goal.updated_at_ms) / 1000));
  return goal.time_used_seconds + live;
}

export function goalStatusLabel(goal: GoalState): string {
  switch (goal.status) {
    case "active":
      return "Goal active";
    case "blocked":
      return "Goal blocked";
    case "budget_limited":
      return "Goal budget reached";
    case "complete":
      return "Goal complete";
  }
}

/** A completed goal is a receipt for the turn that finished it, not permanent
 * composer chrome. Keep it through that answer, then retire it when a later
 * user turn begins. */
export function shouldShowGoalStatus(goal: GoalState, timeline: TimelineItem[]): boolean {
  if (goal.status !== "complete" || !goal.run) return true;
  let lastGoalItem = -1;
  timeline.forEach((item, index) => {
    if ("run" in item && item.run === goal.run) lastGoalItem = index;
  });
  if (lastGoalItem < 0) return true;
  return !timeline.slice(lastGoalItem + 1).some(
    (item) => item.item === "message" && item.role === "user",
  );
}
