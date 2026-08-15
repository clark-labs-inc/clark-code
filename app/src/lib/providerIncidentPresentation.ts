import type { ProviderIncident, TimelineItem } from "../core-bridge/types";

/** A terminal incident card is the canonical presentation for a provider run
 * failure. Suppress the legacy run/session banners for that same run so one
 * failure never appears as three unrelated problems. */
export function hasTerminalProviderIncident(
  timeline: TimelineItem[],
  incidents: Record<string, ProviderIncident>,
  runId: string | undefined,
): boolean {
  if (!runId) return false;
  return timeline.some((item) => {
    if (item.item !== "provider_incident" || item.run !== runId) return false;
    const status = incidents[item.id]?.status;
    return status === "failed" || status === "interrupted";
  });
}
