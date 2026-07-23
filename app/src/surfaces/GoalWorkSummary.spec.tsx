import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { GoalState } from "../core-bridge/types";
import { GoalWorkSummary } from "./GoalWorkSummary";

const completedGoal: GoalState = {
  id: "goal-1",
  objective: "Finish the feature",
  status: "complete",
  run: "run-1",
  tokens_used: 1_000,
  time_used_seconds: 90,
  continuations: 1,
  updated_at_ms: Date.now(),
};

function render(runActive: boolean): string {
  return renderToStaticMarkup(
    <GoalWorkSummary goal={completedGoal} runActive={runActive}>
      <span>Goal work</span>
    </GoalWorkSummary>,
  );
}

describe("GoalWorkSummary", () => {
  it("keeps terminal goal work live until its run actually finishes", () => {
    expect(render(true)).toContain("Working for");
    expect(render(true)).not.toContain("Worked for");
  });

  it("shows the completed receipt after the run finishes", () => {
    expect(render(false)).toContain("Worked for");
    expect(render(false)).not.toContain("Working for");
  });
});
