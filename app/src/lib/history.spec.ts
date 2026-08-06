import { describe, it, expect } from "vitest";
import {
  buildResumeTranscript,
  migratePlanningSnapshot,
  settleRuns,
  snapshotBeforeTimelineItem,
} from "./history";
import type { ProviderIncident, Snapshot, ToolCall } from "../core-bridge/types";

function tool(id: string, status: ToolCall["status"]): ToolCall {
  return { id, title: id, kind: "execute", status, locations: [], content: [] };
}

describe("settleRuns", () => {
  it("cancels live runs and settles in-flight tool calls quietly", () => {
    const snapshot: Snapshot = {
      runs: {
        r1: { id: "r1", status: "running" },
        r2: { id: "r2", status: "done" },
      },
      timeline: [],
      tool_calls: {
        t1: tool("t1", "in_progress"),
        t2: tool("t2", "pending"),
        t3: tool("t3", "failed"),
      },
      artifacts: [],
      provider_incidents: {},
      goal: {
        id: "goal-1",
        objective: "finish the migration",
        status: "active",
        tokens_used: 0,
        time_used_seconds: 0,
        continuations: 0,
        updated_at_ms: 1,
      },
      pending_permission: { id: "p1", session: "s1", title: "allow?", options: [] },
    };
    const settled = settleRuns(snapshot);
    expect(settled.runs.r1.status).toBe("cancelled");
    expect(settled.runs.r2.status).toBe("done");
    // Interrupted tools remain explicitly incomplete instead of being
    // rewritten as successful; real failures are preserved.
    expect(settled.tool_calls.t1.status).toBe("cancelled");
    expect(settled.tool_calls.t2.status).toBe("cancelled");
    expect(settled.tool_calls.t3.status).toBe("failed");
    expect(settled.goal).toMatchObject({
      status: "blocked",
      blocker_reason: "Clark stopped before the goal finished.",
    });
    expect(settled.pending_permission).toBeUndefined();
  });

  it("returns the snapshot unchanged when nothing is live", () => {
    const snapshot: Snapshot = {
      runs: { r1: { id: "r1", status: "done" } },
      timeline: [],
      tool_calls: { t1: tool("t1", "completed") },
      artifacts: [],
      provider_incidents: {},
    };
    expect(settleRuns(snapshot)).toBe(snapshot);
  });

  it("marks an unfinished provider incident interrupted without inventing completion time", () => {
    const incident: ProviderIncident = {
      id: "incident-1",
      status: "retrying",
      scope: "model_request",
      failure_class: "transient_transport",
      category: "connection_lost",
      message: "The model connection was interrupted.",
      detail: "connection closed",
      model: "test-model",
      provider_route: "gateway.test/v1",
      request: {
        idempotency_key: "request-1",
        attempts: 1,
        max_attempts: 4,
        retries: { transient: 1, rate_limit: 0, authentication: 0 },
        output_started: false,
        started_at_ms: 10,
      },
      observed_at_ms: 20,
      updated_at_ms: 20,
    };
    const snapshot: Snapshot = {
      runs: {},
      timeline: [{ item: "provider_incident", run: "r1", id: incident.id }],
      tool_calls: {},
      artifacts: [],
      provider_incidents: { [incident.id]: incident },
    };

    const settled = settleRuns(snapshot);
    expect(settled.provider_incidents[incident.id].status).toBe("interrupted");
    expect(settled.provider_incidents[incident.id].completed_at_ms).toBeUndefined();
  });
});

describe("buildResumeTranscript", () => {
  it("preserves typed messages, tools, and reasoning for provider compaction", () => {
    const snapshot: Snapshot = {
      runs: {},
      timeline: [
        { item: "message", run: "user", role: "user", blocks: [{ type: "text", text: "install node" }] },
        { item: "tool_call", id: "t1" },
        {
          item: "message",
          run: "r1",
          role: "agent",
          blocks: [
            { type: "thinking", text: "hmm" },
            { type: "text", text: "Waiting for brew to finish." },
          ],
        },
      ],
      tool_calls: { t1: { ...tool("t1", "completed"), title: "brew install node", locations: [{ path: "/tmp" }] } },
      artifacts: [],
      provider_incidents: {},
    };
    const out = buildResumeTranscript(snapshot)!;
    expect(out.items[0]).toMatchObject({ item: "message", role: "user" });
    expect(out.items[1]).toMatchObject({
      item: "tool_call",
      title: "brew install node",
      kind: "execute",
      status: "completed",
      locations: [{ path: "/tmp" }],
    });
    expect(JSON.stringify(out)).toContain("Waiting for brew to finish.");
    expect(JSON.stringify(out)).toContain("hmm");
  });

  it("returns null for an empty transcript and keeps every turn for provider compaction", () => {
    expect(
      buildResumeTranscript({ runs: {}, timeline: [], tool_calls: {}, artifacts: [], provider_incidents: {} }),
    ).toBeNull();

    const long: Snapshot = {
      runs: {},
      timeline: Array.from({ length: 50 }, (_, i) => ({
        item: "message" as const,
        run: "user",
        role: "user" as const,
        blocks: [{ type: "text" as const, text: `turn ${i} ${"x".repeat(40)}` }],
      })),
      tool_calls: {},
      artifacts: [],
      provider_incidents: {},
    };
    const out = buildResumeTranscript(long)!;
    expect(out.items).toHaveLength(50);
    expect(out.truncated).toBe(false);
    expect(JSON.stringify(out)).toContain("turn 49");
    expect(JSON.stringify(out)).toContain("turn 0 ");
  });

  it("preserves complete large user turns and typed skill bindings", () => {
    const snapshot: Snapshot = {
      runs: {},
      timeline: [{
        item: "message",
        run: "user",
        role: "user",
        blocks: [
          { type: "text", text: "x".repeat(2_000) },
          {
            type: "skill_reference",
            id: "skill_123",
            revision: "rev_456",
            name: "brainstorming",
          },
        ],
      }],
      tool_calls: {},
      artifacts: [],
      provider_incidents: {},
    };
    const replay = buildResumeTranscript(snapshot)!;
    expect(JSON.stringify(replay)).toContain("x".repeat(2_000));
    expect(replay.items[0]).toMatchObject({
      item: "message",
      blocks: expect.arrayContaining([{
        type: "skill_reference",
        id: "skill_123",
        revision: "rev_456",
        name: "brainstorming",
      }]),
    });
  });

  it("replays the latest proposed plan as typed state", () => {
    const snapshot: Snapshot = {
      runs: {}, timeline: [], tool_calls: {}, artifacts: [], provider_incidents: {},
      proposed_plan: {
        id: "plan-1", revision: 2, markdown: "1. Build it", status: "awaiting_decision",
      },
    };
    expect(buildResumeTranscript(snapshot)?.items).toContainEqual({
      item: "proposed_plan",
      plan: snapshot.proposed_plan,
    });
  });

  it("replays a compacted model checkpoint plus only the later visible tail", () => {
    const snapshot: Snapshot = {
      runs: {},
      timeline: [
        { item: "message", run: "old", role: "user", blocks: [{ type: "text", text: "old turn" }] },
        { item: "message", run: "compact", role: "system", blocks: [{ type: "text", text: "compacted" }] },
        { item: "message", run: "new", role: "user", blocks: [{ type: "text", text: "new turn" }] },
      ],
      model_context_checkpoint: {
        timeline_index: 2,
        transcript: {
          truncated: false,
          items: [{ item: "message", role: "user", blocks: [{ type: "text", text: "summary" }] }],
        },
      },
      tool_calls: {}, artifacts: [], provider_incidents: {},
    };

    const out = buildResumeTranscript(snapshot)!;
    expect(JSON.stringify(out)).toContain("summary");
    expect(JSON.stringify(out)).toContain("new turn");
    expect(JSON.stringify(out)).not.toContain("old turn");
    expect(JSON.stringify(out)).not.toContain("compacted");
  });
});

describe("planning history migration", () => {
  it("upgrades the old overloaded plan shape once at the replay boundary", () => {
    const legacy = {
      runs: {}, tool_calls: {}, artifacts: [],
      plan: { phases: [{ title: "Inspect", status: "in_progress" }] },
      timeline: [{
        item: "plan", run: "r1",
        plan: { phases: [{ title: "Inspect", status: "in_progress" }] },
      }],
    } as unknown as Snapshot;
    const migrated = migratePlanningSnapshot(legacy);
    expect(migrated.execution_checklist?.steps[0].title).toBe("Inspect");
    expect(migrated.timeline[0]).toMatchObject({ item: "execution_checklist", run: "r1" });
    expect("plan" in migrated).toBe(false);
  });
});

describe("snapshotBeforeTimelineItem", () => {
  it("drops the edited turn and every later branch item", () => {
    const snapshot: Snapshot = {
      session: "chat-1",
      runs: {
        r1: { id: "r1", status: "done" },
        r2: { id: "r2", status: "failed" },
      },
      timeline: [
        { item: "message", run: "user", role: "user", blocks: [{ type: "text", text: "first" }] },
        { item: "tool_call", id: "t1" },
        { item: "message", run: "r1", role: "agent", blocks: [{ type: "text", text: "done" }] },
        { item: "artifact", id: "a1" },
        {
          item: "message",
          run: "user",
          role: "user",
          blocks: [{ type: "text", text: "old second" }],
        },
        { item: "tool_call", id: "t2" },
        { item: "message", run: "r2", role: "agent", blocks: [{ type: "text", text: "failed" }] },
        { item: "artifact", id: "a2" },
      ],
      tool_calls: {
        t1: tool("t1", "completed"),
        t2: tool("t2", "failed"),
      },
      artifacts: [
        { id: "a1", title: "kept", kind: "file" },
        { id: "a2", title: "dropped", kind: "file" },
      ],
      provider_incidents: {},
      pending_permission: { id: "p1", session: "chat-1", title: "stale", options: [] },
      focus: { surface: "files", path: "stale.ts" },
      fan_out: { title: "stale", total: 1, done: 0, running: 1, agents: [] },
    };

    const prefix = snapshotBeforeTimelineItem(snapshot, 4);

    expect(prefix.timeline).toHaveLength(4);
    expect(
      prefix.timeline.some(
        (item) =>
          item.item === "message" &&
          item.role === "user" &&
          item.blocks.some(
            (block) => block.type === "text" && block.text === "old second",
          ),
      ),
    ).toBe(false);
    expect(Object.keys(prefix.runs)).toEqual(["r1"]);
    expect(Object.keys(prefix.tool_calls)).toEqual(["t1"]);
    expect(prefix.artifacts.map((artifact) => artifact.id)).toEqual(["a1"]);
    expect(prefix.pending_permission).toBeUndefined();
    expect(prefix.focus).toBeUndefined();
    expect(prefix.fan_out).toBeUndefined();
  });

  it("keeps a checkpoint only when editing a turn after its boundary", () => {
    const snapshot: Snapshot = {
      runs: {},
      timeline: [
        { item: "message", run: "old", role: "user", blocks: [{ type: "text", text: "old" }] },
        { item: "message", run: "notice", role: "system", blocks: [{ type: "text", text: "compacted" }] },
        { item: "message", run: "new", role: "user", blocks: [{ type: "text", text: "new" }] },
      ],
      model_context_checkpoint: {
        timeline_index: 2,
        transcript: { items: [{ item: "message", role: "user", blocks: [{ type: "text", text: "summary" }] }], truncated: false },
      },
      tool_calls: {}, artifacts: [], provider_incidents: {},
    };

    expect(snapshotBeforeTimelineItem(snapshot, 2).model_context_checkpoint).toBeDefined();
    expect(snapshotBeforeTimelineItem(snapshot, 0).model_context_checkpoint).toBeUndefined();
  });
});
