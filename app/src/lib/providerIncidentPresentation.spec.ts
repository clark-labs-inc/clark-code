import { describe, expect, it } from "vitest";

import type { ProviderIncident, TimelineItem } from "../core-bridge/types";
import { hasTerminalProviderIncident } from "./providerIncidentPresentation";

function incident(status: ProviderIncident["status"]): ProviderIncident {
  return {
    id: "run-2:provider-incident:1",
    status,
    scope: "model_request",
    failure_class: "transient_transport",
    category: "upstream_unavailable",
    message: "temporarily unavailable",
    detail: "502",
    model: "test-model",
    provider_route: "test-route",
    request: {
      idempotency_key: "request-1",
      attempts: 4,
      max_attempts: 4,
      retries: { transient: 3, rate_limit: 0, authentication: 0 },
      output_started: false,
      started_at_ms: 1,
    },
    observed_at_ms: 1,
    updated_at_ms: 2,
  };
}

const timeline: TimelineItem[] = [
  { item: "provider_incident", run: "run-2", id: "run-2:provider-incident:1" },
];

describe("hasTerminalProviderIncident", () => {
  it("matches only a terminal incident belonging to the failed run", () => {
    expect(hasTerminalProviderIncident(timeline, { [incident("failed").id]: incident("failed") }, "run-2"))
      .toBe(true);
    expect(hasTerminalProviderIncident(timeline, { [incident("retrying").id]: incident("retrying") }, "run-2"))
      .toBe(false);
    expect(hasTerminalProviderIncident(timeline, { [incident("failed").id]: incident("failed") }, "run-1"))
      .toBe(false);
  });
});
