import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { ProviderIncident } from "../core-bridge/types";
import { ProviderIncidentCard } from "./ProviderIncidentCard";

const incident: ProviderIncident = {
  id: "run-1:provider-incident:1",
  status: "retrying",
  scope: "model_request",
  failure_class: "transient_transport",
  category: "timeout",
  message: "Model connection timed out while Clark was working.",
  detail: "model endpoint returned 524",
  model: "z-ai/glm-5.2",
  provider_route: "api.clarkslabs.com",
  provider_status: 524,
  provider_error_type: "upstream_timeout",
  request: {
    idempotency_key: "clark-code-request-1",
    provider_request_id: "upstream-1",
    attempts: 4,
    max_attempts: 17,
    retries: { transient: 3, rate_limit: 0, authentication: 0 },
    output_started: false,
    started_at_ms: 1_000,
  },
  execution_recovery: {
    attempt: 2,
    max_attempts: 2,
    started_at_ms: 2_100,
    boundary: {
      execution_id: "run-1",
      attempt_sequence: 1,
      event_sequence: 52,
      transcript_commit_id: "run-1:transcript-commit:52",
      completed_tools: 49,
      last_completed_tool_id: "1:call-49",
      last_completed_tool_name: "update_plan",
      baseline_checkpoint_id: "git-checkpoint",
    },
  },
  observed_at_ms: 2_000,
  updated_at_ms: 2_100,
};

function render(value: ProviderIncident, onContinue?: () => void) {
  return renderToStaticMarkup(createElement(ProviderIncidentCard, {
    incident: value,
    onContinue,
    modelRouteLabel: "Clark's cloud model gateway",
  }));
}

describe("ProviderIncidentCard", () => {
  it("explains execution recovery and the local/cloud boundary", () => {
    const markup = render(incident);
    expect(markup).toContain("Completed tools: 49");
    expect(markup).toContain("Retrying attempt 2 of 2");
    expect(markup).toContain("Files and tools run on this computer");
    expect(markup).toContain("Clark&#x27;s cloud model gateway");
    expect(markup).toContain("upstream-1");
    expect(markup).toContain("run-1:transcript-commit:52");
  });

  it("shows request-local retry progress before execution recovery", () => {
    const markup = render({ ...incident, execution_recovery: undefined, request: { ...incident.request, attempts: 1 } });
    expect(markup).toContain("Retrying request 2 of 17");
  });

  it("updates to recovered duration without offering continuation", () => {
    const markup = render({ ...incident, status: "recovered", completed_at_ms: 14_100 }, vi.fn());
    expect(markup).toContain("Recovered after 12 seconds");
    expect(markup).not.toContain("Continue from saved progress");
  });

  it("offers honest continuation after recovery fails", () => {
    const markup = render({ ...incident, status: "failed", completed_at_ms: 6_000 }, vi.fn());
    expect(markup).toContain("Recovery failed");
    expect(markup).toContain("Continue from saved progress");
  });

  it("does not invent a completion duration for interrupted recovery", () => {
    const markup = render({ ...incident, status: "interrupted" }, vi.fn());
    expect(markup).toContain("stopped before recovery completed");
    expect(markup).not.toContain("Recovered after");
  });
});
