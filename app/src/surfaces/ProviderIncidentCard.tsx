import { Clock3, RotateCcw } from "lucide-react";

import type { ProviderIncident } from "../core-bridge/types";

/**
 * Transient provider incidents are recovery implementation details. The
 * conversation's ordinary Working row stays visible while they retry, and a
 * recovered incident leaves no error-shaped artifact in the transcript.
 *
 * Only a terminal incident needs a user-facing affordance. Even then, it is
 * paused saved work rather than an error: routes, attempt counters, request
 * IDs, and provider diagnostics never belong in the conversation.
 */
export function ProviderIncidentCard({
  incident,
  onContinue,
}: {
  incident: ProviderIncident;
  onContinue?: () => void;
  executionLocation?: "this computer" | "your remote host";
  modelRouteLabel?: string;
}) {
  const terminal = incident.status === "failed" || incident.status === "interrupted";
  if (!terminal || !onContinue) return null;
  const serviceUnavailable = Boolean(
    incident.provider_status && incident.provider_status >= 500,
  ) || incident.provider_error_type === "upstream_unavailable";
  const title = serviceUnavailable
    ? "Clark service was unavailable."
    : incident.category === "rate_limit"
      ? "The model is busy right now."
      : incident.category === "timeout"
        ? "The model did not respond in time."
        : "The run paused before it could finish.";
  const detail = serviceUnavailable
    ? "Your local work is saved. Resume this task when the service is back."
    : "Your local work is saved. Resume this task when you’re ready.";

  return (
    <section
      aria-label="Agent paused"
      aria-live="polite"
      className="rounded-xl border border-border-subtle bg-bg-elevated/70 px-3.5 py-3 text-sm"
    >
      <div className="flex items-start gap-2.5">
        <Clock3 className="mt-0.5 size-4 shrink-0 text-ink-muted" aria-hidden />
        <div className="min-w-0 flex-1">
          <p className="font-medium text-ink">{title}</p>
          <p className="mt-0.5 text-ink-muted">{detail}</p>
          <button
            type="button"
            onClick={onContinue}
            className="mt-3 inline-flex items-center gap-1.5 rounded-lg border border-border bg-bg-elevated px-2.5 py-1.5 text-xs font-medium text-ink-secondary transition hover:bg-bg-hover hover:text-ink"
          >
            <RotateCcw className="size-3.5" aria-hidden />
            Resume task
          </button>
        </div>
      </div>
    </section>
  );
}
