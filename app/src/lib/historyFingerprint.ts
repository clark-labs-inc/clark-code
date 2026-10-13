import type { Snapshot } from "../core-bridge/types";

const RETAINED_HEAD_ITEMS = 160;

/** Bound renderer no-op hashing to the retained head. Native owns the actual
 * page plan and rejects a cursor that cannot be reconciled with the snapshot. */
export function historyFingerprintHead(snapshot: Snapshot, alreadySealedThrough: number): Snapshot {
  const base = snapshot.timeline_offset ?? 0;
  const localStart = alreadySealedThrough - base;
  const checkpoint = snapshot.model_context_checkpoint?.timeline_index;
  const active = snapshot.starting || Object.values(snapshot.runs).some((run) => (
    run.status === "queued" || run.status === "running" || run.status === "awaiting_input"
  ));
  if (localStart < 0 || localStart > snapshot.timeline.length) {
    return snapshot;
  }
  const compactedEnd = Math.min(snapshot.timeline.length, Math.max(0, (checkpoint ?? base) - base));
  const sealEnd = active ? localStart : Math.max(localStart, compactedEnd - RETAINED_HEAD_ITEMS);
  if (sealEnd === 0) return snapshot;

  const timeline = snapshot.timeline.slice(sealEnd);
  const toolIds = new Set(timeline.flatMap((item) => item.item === "tool_call" ? [item.id] : []));
  const artifactIds = new Set(timeline.flatMap((item) => item.item === "artifact" ? [item.id] : []));
  const incidentIds = new Set(timeline.flatMap((item) => item.item === "provider_incident" ? [item.id] : []));
  return {
    ...snapshot,
    timeline_offset: base + sealEnd,
    timeline,
    tool_calls: Object.fromEntries(Object.entries(snapshot.tool_calls).filter(([id]) => toolIds.has(id))),
    artifacts: snapshot.artifacts.filter((artifact) => artifactIds.has(artifact.id)),
    provider_incidents: Object.fromEntries(
      Object.entries(snapshot.provider_incidents).filter(([id]) => incidentIds.has(id)),
    ),
  };
}
