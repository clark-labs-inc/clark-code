import { describe, expect, it } from "vitest";
import { emptySnapshot, type Snapshot } from "../core-bridge/types";
import { shouldResumeSavedProgress } from "./sessionStore.conversationActions";

function interruptedActiveGoal(): Snapshot {
  return {
    ...emptySnapshot(),
    goal: {
      id: "goal-1",
      objective: "finish the work",
      status: "active",
      run: "run-1",
      tokens_used: 100,
      time_used_seconds: 30,
      continuations: 1,
      updated_at_ms: 10,
    },
    runs: {
      "run-1": {
        id: "run-1",
        status: "failed",
        outcome: {
          status: "failed",
          failure_kind: "runtime_interrupted",
        },
      },
    },
  };
}

describe("saved standing-goal continuation", () => {
  it("resumes an active goal whose owning run was interrupted", () => {
    expect(shouldResumeSavedProgress(interruptedActiveGoal())).toBe(true);
  });

  it("does not resume when the goal authority says blocked", () => {
    const snapshot = interruptedActiveGoal();
    snapshot.goal!.status = "blocked";
    expect(shouldResumeSavedProgress(snapshot)).toBe(false);
  });

  it("does not turn other typed failures into automatic retries", () => {
    const snapshot = interruptedActiveGoal();
    snapshot.runs["run-1"].outcome!.failure_kind = "tool_fatal";
    expect(shouldResumeSavedProgress(snapshot)).toBe(false);
  });
});
