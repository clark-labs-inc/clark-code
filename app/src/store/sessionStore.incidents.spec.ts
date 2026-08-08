import { describe, expect, it } from "vitest";

import { emptySnapshot, normalizeSnapshot, type ProviderIncident } from "../core-bridge/types";
import { mergeHistory } from "./sessionStore";

const incident: ProviderIncident = {
  id: "run-1:provider-incident:1",
  status: "failed",
  scope: "model_request",
  failure_class: "transient_transport",
  category: "timeout",
  message: "Model connection timed out.",
  detail: "gateway timeout",
  model: "test-model",
  provider_route: "gateway.test",
  request: {
    idempotency_key: "request-1",
    attempts: 4,
    max_attempts: 17,
    retries: { transient: 3, rate_limit: 0, authentication: 0 },
    output_started: false,
    started_at_ms: 1,
  },
  observed_at_ms: 2,
  updated_at_ms: 3,
  completed_at_ms: 3,
};

describe("provider incident snapshot boundaries", () => {
  it("normalizes legacy snapshots once at ingress", () => {
    const normalized = normalizeSnapshot({ runs: {}, timeline: [], tool_calls: {}, artifacts: [] });
    expect(normalized.provider_incidents).toEqual({});
  });

  it("preserves restored incidents when live history is merged", () => {
    const prefix = emptySnapshot();
    prefix.timeline.push({ item: "provider_incident", run: "run-1", id: incident.id });
    prefix.provider_incidents[incident.id] = incident;

    const merged = mergeHistory(prefix, { ...emptySnapshot(), session: "conversation-1" });
    expect(merged.timeline).toContainEqual({
      item: "provider_incident",
      run: "run-1",
      id: incident.id,
    });
    expect(merged.provider_incidents[incident.id]).toEqual(incident);
  });

  it("offsets a live compaction checkpoint across the restored prefix", () => {
    const prefix = emptySnapshot();
    prefix.timeline.push({
      item: "message",
      run: "old",
      role: "user",
      blocks: [{ type: "text", text: "old turn" }],
    });
    const live = emptySnapshot();
    live.timeline.push({
      item: "message",
      run: "compact",
      role: "system",
      blocks: [{ type: "text", text: "compacted" }],
    });
    live.model_context_checkpoint = {
      timeline_index: 1,
      transcript: {
        truncated: false,
        items: [{ item: "message", role: "user", blocks: [{ type: "text", text: "summary" }] }],
      },
    };

    expect(mergeHistory(prefix, live).model_context_checkpoint?.timeline_index).toBe(2);
  });

  it("preserves both runs when a resumed provider reuses a legacy run id", () => {
    const prefix = emptySnapshot();
    prefix.runs["run-1"] = { id: "run-1", status: "done" };
    prefix.timeline.push({
      item: "message",
      run: "run-1",
      role: "agent",
      phase: "final_answer",
      blocks: [{ type: "text", text: "old answer" }],
    });
    const live = emptySnapshot();
    live.runs["run-1"] = {
      id: "run-1",
      status: "running",
      outcome: {
        status: "running",
        execution: {
          execution_id: "conversation:run-1",
          root_path: "/tmp/project",
          attempts: 1,
          recoveries: 0,
          child_executions: 0,
          completed_children: 0,
          failed_children: 0,
          weighted_tokens: 0,
          cost_usd: 0,
          changed_paths: [],
          completed_tools: [],
          failed_tools: [],
        },
      },
    };
    live.timeline.push({
      item: "message",
      run: "run-1",
      role: "agent",
      blocks: [{ type: "text", text: "new work" }],
    });

    const merged = mergeHistory(prefix, live);

    expect(merged.runs["run-1"].status).toBe("done");
    expect(merged.runs["run-1~resume-1"]).toMatchObject({
      id: "run-1~resume-1",
      status: "running",
    });
    expect(merged.runs["run-1~resume-1"].outcome?.execution?.execution_id)
      .toBe("conversation:run-1~resume-1");
    expect(merged.timeline.map((item) => "run" in item ? item.run : undefined))
      .toEqual(["run-1", "run-1~resume-1"]);
  });
});
