import { describe, expect, it } from "vitest";
import { currentActivity } from "./activity";
import { emptySnapshot, type Snapshot } from "../core-bridge/types";

function withRun(status: Snapshot["runs"][string]["status"]): Snapshot {
  const s = emptySnapshot();
  s.runs["r1"] = { id: "r1", status };
  return s;
}

describe("currentActivity", () => {
  it("is idle/ready with no running run", () => {
    const a = currentActivity(emptySnapshot());
    expect(a.busy).toBe(false);
    expect(a.label).toBe("Ready");
  });

  it("surfaces the in-progress tool as the current activity", () => {
    const s = withRun("running");
    s.tool_calls["t"] = {
      id: "t", title: "Edit notes.txt", kind: "edit", status: "in_progress",
      locations: [{ path: "/w/notes.txt" }], content: [],
    };
    const a = currentActivity(s);
    expect(a.busy).toBe(true);
    expect(a.label).toBe("Edit notes.txt");
    expect(a.detail).toBe("/w/notes.txt");
  });

  it("falls back to the in-progress plan phase, then Thinking", () => {
    const s = withRun("running");
    s.plan = { phases: [
      { title: "Step A", status: "completed" },
      { title: "Step B", status: "in_progress" },
    ] };
    const a = currentActivity(s);
    expect(a.label).toBe("Step B");
    expect(a.steps).toEqual({ done: 1, total: 2 });
    expect(a.progress).toBeCloseTo(0.5);

    const s2 = withRun("running");
    expect(currentActivity(s2).label).toBe("Thinking…");
  });

  it("reports failure", () => {
    const a = currentActivity(withRun("failed"));
    expect(a.failed).toBe(true);
    expect(a.label).toBe("Run failed");
  });
});
