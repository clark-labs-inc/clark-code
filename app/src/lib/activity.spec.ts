import { describe, expect, it } from "vitest";
import { currentActivity, executionDiagnostic, shouldShowPending } from "./activity";
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
    s.execution_checklist = { revision: 1, steps: [
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

  it("keeps pending visible after commentary while the run continues", () => {
    const beforeResponse = withRun("running");
    beforeResponse.timeline.push({
      item: "message",
      run: "r1",
      role: "user",
      blocks: [{ type: "text", text: "Help me" }],
    });
    expect(shouldShowPending(beforeResponse)).toBe(true);

    const reasoningStarted = structuredClone(beforeResponse);
    reasoningStarted.timeline.push({
      item: "message",
      run: "r1",
      role: "agent",
      blocks: [{ type: "thinking", text: "Inspecting the request" }],
    });
    expect(shouldShowPending(reasoningStarted)).toBe(false);

    const answerStarted = structuredClone(beforeResponse);
    answerStarted.timeline.push({
      item: "message",
      run: "r1",
      role: "agent",
      blocks: [{ type: "text", text: "I found it" }],
    });
    expect(shouldShowPending(answerStarted)).toBe(false);

    const commentaryFinished = structuredClone(beforeResponse);
    commentaryFinished.timeline.push({
      item: "message",
      run: "r1",
      role: "agent",
      phase: "commentary",
      blocks: [{ type: "text", text: "Coverage is complete. Writing the report now." }],
    });
    expect(shouldShowPending(commentaryFinished)).toBe(true);

    const finalAnswer = structuredClone(beforeResponse);
    finalAnswer.timeline.push({
      item: "message",
      run: "r1",
      role: "agent",
      phase: "final_answer",
      blocks: [{ type: "text", text: "Here is the report." }],
    });
    expect(shouldShowPending(finalAnswer)).toBe(false);
  });

  it("shows pending after a completed timeline item while the next step starts", () => {
    const snapshot = withRun("running");
    snapshot.timeline.push({ item: "tool_call", id: "done", run: "r1" });
    snapshot.tool_calls.done = {
      id: "done",
      title: "Inspect files",
      kind: "read",
      status: "completed",
      locations: [],
      content: [],
    };
    expect(shouldShowPending(snapshot)).toBe(true);
  });

  it("lets an active tool own the pending state", () => {
    const snapshot = withRun("running");
    snapshot.tool_calls.tool = {
      id: "tool",
      title: "Inspect files",
      kind: "read",
      status: "in_progress",
      locations: [],
      content: [],
    };
    expect(shouldShowPending(snapshot)).toBe(false);
  });

  it("lets the structured incident card own retry progress", () => {
    const snapshot = withRun("running");
    snapshot.timeline.push({ item: "provider_incident", run: "r1", id: "incident-1" });
    snapshot.provider_incidents = {
      "incident-1": {
        id: "incident-1",
        status: "retrying",
        scope: "model_request",
        failure_class: "transient_transport",
        category: "timeout",
        message: "Model connection timed out.",
        detail: "gateway timeout",
        model: "test-model",
        provider_route: "gateway.test",
        request: {
          idempotency_key: "request-1",
          attempts: 1,
          max_attempts: 17,
          retries: { transient: 1, rate_limit: 0, authentication: 0 },
          output_started: false,
          started_at_ms: 1,
        },
        observed_at_ms: 2,
        updated_at_ms: 3,
      },
    };
    expect(shouldShowPending(snapshot)).toBe(false);
  });
});

describe("executionDiagnostic", () => {
  it("summarizes typed root lifecycle evidence without parsing prose", () => {
    expect(executionDiagnostic({
      status: "done",
      execution: {
        execution_id: "session:run-1",
        root_path: "/root",
        attempts: 2,
        recoveries: 1,
        child_executions: 0,
        completed_children: 0,
        failed_children: 0,
        weighted_tokens: 120,
        cost_usd: 0.01,
        changed_paths: ["src/lib.rs"],
        completed_tools: ["edit_file"],
        failed_tools: [],
      },
    })).toBe("Root execution: 2 attempts · 1 recovered interruption · 1 changed path");
  });

  it("stays absent for providers without a lifecycle receipt", () => {
    expect(executionDiagnostic({ status: "done" })).toBeUndefined();
  });
});
