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
});
