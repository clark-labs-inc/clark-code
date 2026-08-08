import { describe, expect, it } from "vitest";
import type { GoalState, TimelineItem } from "../core-bridge/types";
import { shouldShowGoalStatus } from "./goal";

const completedGoal: GoalState = {
  id: "goal-1",
  objective: "Finish the feature",
  status: "complete",
  run: "goal-run",
  tokens_used: 1_000,
  time_used_seconds: 30,
  continuations: 1,
  updated_at_ms: 1,
};

function message(run: string, role: "user" | "agent"): TimelineItem {
  return { item: "message", run, role, blocks: [{ type: "text", text: role }] };
}

describe("shouldShowGoalStatus", () => {
  it("keeps the completion receipt through the goal's final answer", () => {
    expect(shouldShowGoalStatus(completedGoal, [
      message("goal-run", "user"),
      message("goal-run", "agent"),
    ])).toBe(true);
  });

  it("retires the completion receipt when a later user turn begins", () => {
    expect(shouldShowGoalStatus(completedGoal, [
      message("goal-run", "user"),
      message("goal-run", "agent"),
      message("follow-up-run", "user"),
    ])).toBe(false);
  });

  it("does not mistake earlier conversation turns for a follow-up", () => {
    expect(shouldShowGoalStatus(completedGoal, [
      message("earlier-run", "user"),
      message("goal-run", "user"),
      message("goal-run", "agent"),
    ])).toBe(true);
  });

  it("keeps non-complete goal states visible", () => {
    expect(shouldShowGoalStatus(
      { ...completedGoal, status: "blocked" },
      [message("follow-up-run", "user")],
    )).toBe(true);
  });
});
