import { CheckCircle2, Cloud, RotateCcw, TriangleAlert } from "lucide-react";

import type {
  ProviderIncident,
  ProviderIncidentCategory,
  ProviderIncidentScope,
} from "../core-bridge/types";
import { cn } from "../lib/cn";

const CATEGORY_LABEL: Record<ProviderIncidentCategory, string> = {
  timeout: "Timeout",
  rate_limit: "Rate limit",
  upstream_unavailable: "Upstream unavailable",
  connection_lost: "Connection lost",
};

const SCOPE_LABEL: Record<ProviderIncidentScope, string> = {
  model_request: "Model request",
  provider_event_stream: "Provider event stream",
  provider_process: "Provider process",
  cloud_history_sync: "Cloud history sync",
  tool_execution_host: "Tool execution host",
};

function elapsedSeconds(incident: ProviderIncident): number | null {
  if (incident.completed_at_ms === undefined) return null;
  const start = incident.execution_recovery?.started_at_ms ?? incident.observed_at_ms;
  return Math.max(0, Math.round((incident.completed_at_ms - start) / 1000));
}

function outcome(incident: ProviderIncident): string {
  const seconds = elapsedSeconds(incident);
  switch (incident.status) {
    case "observed":
      return "Checking recovery options…";
    case "retrying": {
      const recovery = incident.execution_recovery;
      if (recovery) return `Retrying attempt ${recovery.attempt} of ${recovery.max_attempts}…`;
      const next = Math.min(incident.request.attempts + 1, incident.request.max_attempts);
      return `Retrying request ${next} of ${incident.request.max_attempts}…`;
    }
    case "recovered":
      return `Recovered${seconds === null ? "" : ` after ${seconds} second${seconds === 1 ? "" : "s"}`}.`;
    case "interrupted":
      return "the agent stopped before recovery completed.";
    case "failed":
      return "Recovery failed.";
  }
}

export function ProviderIncidentCard({
  incident,
  onContinue,
  executionLocation = "this computer",
  modelRouteLabel = "the selected model provider",
}: {
  incident: ProviderIncident;
  onContinue?: () => void;
  executionLocation?: "this computer" | "your remote host";
  modelRouteLabel?: string;
}) {
  const recovered = incident.status === "recovered";
  const terminalFailure = incident.status === "failed" || incident.status === "interrupted";
  const boundary = incident.execution_recovery?.boundary;
  const retries = incident.request.retries;

  return (
    <section
      aria-label="Provider incident"
      aria-live="polite"
      className={cn(
        "rounded-xl border px-3.5 py-3 text-sm",
        terminalFailure
          ? "border-danger/35 bg-danger/6"
          : recovered
            ? "border-success/30 bg-success/5"
            : "border-border bg-bg-secondary",
      )}
    >
      <div className="flex items-start gap-2.5">
        {recovered ? (
          <CheckCircle2 className="mt-0.5 size-4 shrink-0 text-success" aria-hidden />
        ) : terminalFailure ? (
          <TriangleAlert className="mt-0.5 size-4 shrink-0 text-danger" aria-hidden />
        ) : (
          <Cloud className="mt-0.5 size-4 shrink-0 text-accent" aria-hidden />
        )}
        <div className="min-w-0 flex-1">
          <p className="font-medium text-ink">{incident.message}</p>
          <p className="mt-0.5 text-ink-muted">
            {boundary && boundary.completed_tools > 0 && (
              <span>Completed tools: {boundary.completed_tools} · </span>
            )}
            {outcome(incident)}
          </p>
          <p className="mt-2 text-xs leading-relaxed text-ink-faint">
            Files and tools run on {executionLocation}. Model requests use {modelRouteLabel}.
          </p>
          <details className="mt-2 text-xs text-ink-muted">
            <summary className="cursor-pointer select-none font-medium text-ink-secondary">
              Details
            </summary>
            <dl className="mt-2 grid grid-cols-[max-content_minmax(0,1fr)] gap-x-3 gap-y-1 rounded-lg bg-bg-sunken/60 p-2 font-mono [overflow-wrap:anywhere]">
              <dt>Scope</dt><dd>{SCOPE_LABEL[incident.scope]}</dd>
              <dt>Category</dt><dd>{CATEGORY_LABEL[incident.category]}</dd>
              <dt>Failure class</dt><dd>{incident.failure_class}</dd>
              <dt>Route</dt><dd>{incident.provider_route}</dd>
              <dt>Model</dt><dd>{incident.model}</dd>
              <dt>Request attempts</dt><dd>{incident.request.attempts} / {incident.request.max_attempts}</dd>
              <dt>Retries</dt><dd>transient {retries.transient}, rate limit {retries.rate_limit}, auth {retries.authentication}</dd>
              <dt>Output started</dt><dd>{incident.request.output_started ? "yes" : "no"}</dd>
              <dt>Client idempotency key</dt><dd>{incident.request.idempotency_key}</dd>
              {incident.request.provider_request_id && <><dt>Provider request ID</dt><dd>{incident.request.provider_request_id}</dd></>}
              {incident.provider_status !== undefined && <><dt>Status</dt><dd>{incident.provider_status}</dd></>}
              {incident.provider_error_type && <><dt>Error type</dt><dd>{incident.provider_error_type}</dd></>}
              {boundary && <>
                <dt>Execution</dt><dd>{boundary.execution_id}</dd>
                <dt>Attempt boundary</dt><dd>{boundary.attempt_sequence}</dd>
                <dt>Event boundary</dt><dd>{boundary.event_sequence}</dd>
                <dt>Transcript commit</dt><dd>{boundary.transcript_commit_id}</dd>
                {boundary.last_completed_tool_name && <><dt>Last completed tool</dt><dd>{boundary.last_completed_tool_name}</dd></>}
                {boundary.last_completed_tool_id && <><dt>Last tool ID</dt><dd>{boundary.last_completed_tool_id}</dd></>}
                {boundary.baseline_checkpoint_id && <><dt>Baseline checkpoint</dt><dd>{boundary.baseline_checkpoint_id}</dd></>}
              </>}
              <dt>Request started</dt><dd>{new Date(incident.request.started_at_ms).toLocaleString()}</dd>
              <dt>Observed</dt><dd>{new Date(incident.observed_at_ms).toLocaleString()}</dd>
              <dt>Last update</dt><dd>{new Date(incident.updated_at_ms).toLocaleString()}</dd>
              {incident.completed_at_ms !== undefined && <><dt>Completed</dt><dd>{new Date(incident.completed_at_ms).toLocaleString()}</dd></>}
              <dt>Provider detail</dt><dd>{incident.detail}</dd>
            </dl>
          </details>
          {terminalFailure && onContinue && (
            <button
              type="button"
              onClick={onContinue}
              className="mt-3 inline-flex items-center gap-1.5 rounded-lg border border-border bg-bg-elevated px-2.5 py-1.5 text-xs font-medium text-ink-secondary transition hover:bg-bg-hover hover:text-ink"
            >
              <RotateCcw className="size-3.5" aria-hidden />
              Continue from saved progress
            </button>
          )}
        </div>
      </div>
    </section>
  );
}
