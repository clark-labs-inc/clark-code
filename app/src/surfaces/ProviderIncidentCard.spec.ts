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
  message: "Model connection timed out while the agent was working.",
  detail: "model endpoint returned 524",
  model: "z-ai/glm-5.2",
  provider_route: "api.product.example",
  provider_status: 524,
  provider_error_type: "upstream_timeout",
  request: {
    idempotency_key: "example-request-1",
    provider_request_id: "upstream-1",
    attempts: 4,
    max_attempts: 17,
    retries: { transient: 3, rate_limit: 0, authentication: 0 },
    output_started: false,
    started_at_ms: 1_000,
  },
  execution_recovery: {
    attempt: 2,
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
  }));
}

describe("ProviderIncidentCard", () => {
  it.each(["observed", "retrying", "recovered"] as const)(
    "keeps %s recovery invisible",
    (status) => {
      const markup = render({ ...incident, status });
      expect(markup).toBe("");
      expect(markup).not.toContain("Retrying");
      expect(markup).not.toContain("upstream-1");
    },
  );

  it.each(["failed", "interrupted"] as const)(
    "offers a quiet continuation after terminal %s recovery",
    (status) => {
      const markup = render({ ...incident, status }, vi.fn());
      expect(markup).toContain("Clark service was unavailable");
      expect(markup).toContain("Your local work is saved");
      expect(markup).toContain("Resume task");
      expect(markup).not.toContain("danger");
      expect(markup).not.toContain("error");
      expect(markup).not.toContain("Retrying");
      expect(markup).not.toContain("attempt");
      expect(markup).not.toContain("gateway");
      expect(markup).not.toContain("upstream-1");
      expect(markup).not.toContain("Details");
    },
  );

  it("hides terminal incidents that are no longer the active recovery point", () => {
    expect(render({ ...incident, status: "failed" })).toBe("");
  });
});
