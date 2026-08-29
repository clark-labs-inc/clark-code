import { describe, expect, it } from "vitest";
import {
  currentActivity,
  isAwaitingAssistantReply,
  isThinkingOnlyMessage,
  shouldShowPending,
} from "./activity";
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

  it("stays busy while a prompt is starting even before a run exists", () => {
    const s = emptySnapshot();
    s.starting = true;
    s.timeline.push({
      item: "message",
      run: "starting",
      role: "user",
      blocks: [{ type: "text", text: "Review this screenshot" }],
    });
    const a = currentActivity(s);
    expect(a.busy).toBe(true);
    expect(a.label).toBe("Thinking…");
    // The upload gap must keep the working row visible, not a static message.
    expect(shouldShowPending(s)).toBe(true);
  });

  it("returns to idle once starting has been retired", () => {
    const s = emptySnapshot();
    s.starting = false;
    expect(currentActivity(s).busy).toBe(false);
    expect(shouldShowPending(s)).toBe(false);
  });

  it("surfaces the in-progress tool as the current activity", () => {
    const s = withRun("running");
    s.tool_calls["t"] = {
      id: "t", title: "Edit notes.txt", kind: "edit", status: "in_progress",
      locations: [{ path: "/w/notes.txt" }], content: [],
    };
    s.timeline.push({ item: "tool_call", id: "t", run: "r1" });
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
    s.timeline.push({
      item: "execution_checklist",
      run: "r1",
      checklist: s.execution_checklist,
    });
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

  it("does not report an incomplete post-answer verification as a failed answer", () => {
    const snapshot = withRun("failed");
    snapshot.runs.r1.outcome = {
      status: "failed",
      failure_kind: "verification_incomplete",
    };
    const activity = currentActivity(snapshot);
    expect(activity.failed).toBeUndefined();
    expect(activity.label).toBe("Verification incomplete");
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
    expect(isThinkingOnlyMessage(reasoningStarted.timeline.at(-1))).toBe(true);
    expect(shouldShowPending(reasoningStarted)).toBe(true);
    expect(currentActivity(reasoningStarted).label).toBe("Working…");

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
    expect(isAwaitingAssistantReply(commentaryFinished.timeline)).toBe(true);

    const finalAnswer = structuredClone(beforeResponse);
    finalAnswer.timeline.push({
      item: "message",
      run: "r1",
      role: "agent",
      phase: "final_answer",
      blocks: [{ type: "text", text: "Here is the report." }],
    });
    expect(shouldShowPending(finalAnswer)).toBe(false);
    expect(isAwaitingAssistantReply(finalAnswer.timeline)).toBe(false);
  });

  it("keeps the reply reserve through plans and reasoning until prose begins", () => {
    const snapshot = withRun("running");
    snapshot.timeline.push(
      {
        item: "message",
        run: "r1",
        role: "user",
        blocks: [{ type: "text", text: "Explain this project" }],
      },
      {
        item: "execution_checklist",
        run: "r1",
        checklist: { revision: 1, steps: [{ title: "Inspect", status: "in_progress" }] },
      },
      {
        item: "message",
        run: "r1",
        role: "agent",
        blocks: [{ type: "thinking", text: "Tracing the entrypoint" }],
      },
    );

    expect(isAwaitingAssistantReply(snapshot.timeline)).toBe(true);

    snapshot.timeline.push({
      item: "message",
      run: "r1",
      role: "agent",
      blocks: [{ type: "text", text: "The entrypoint is in src/main.rs." }],
    });
    expect(isAwaitingAssistantReply(snapshot.timeline)).toBe(false);
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
    expect(currentActivity(snapshot).label).toBe("Working…");
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
    snapshot.timeline.push({ item: "tool_call", id: "tool", run: "r1" });
    expect(shouldShowPending(snapshot)).toBe(false);
  });

  it("does not reuse stale pre-compaction work as the current run label", () => {
    const snapshot = emptySnapshot();
    snapshot.runs.previous = { id: "previous", status: "done" };
    snapshot.runs.current = { id: "current", status: "running" };
    snapshot.tool_calls.oldTool = {
      id: "oldTool",
      title: "Provision GPU box",
      kind: "execute",
      status: "in_progress",
      locations: [],
      content: [],
    };
    snapshot.execution_checklist = {
      revision: 1,
      steps: [{ title: "Provision GPU box: build toolchain", status: "in_progress" }],
    };
    snapshot.timeline.push(
      { item: "tool_call", id: "oldTool", run: "previous" },
      {
        item: "execution_checklist",
        run: "previous",
        checklist: snapshot.execution_checklist,
      },
      {
        item: "message",
        run: "current",
        role: "user",
        blocks: [{ type: "text", text: "What does sealed mean?" }],
      },
    );
    snapshot.model_context_checkpoint = {
      transcript: { items: [], truncated: true },
      timeline_index: 2,
    };

    expect(currentActivity(snapshot)).toMatchObject({
      busy: true,
      label: "Thinking…",
    });
    expect(currentActivity(snapshot).steps).toBeUndefined();
    expect(shouldShowPending(snapshot)).toBe(true);
  });

  it("keeps ordinary pending progress visible while recovery stays quiet", () => {
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
    expect(shouldShowPending(snapshot)).toBe(true);
    expect(currentActivity(snapshot).label).toBe("Working…");
  });
});
