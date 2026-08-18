import type { Snapshot, TimelineItem, TranscriptPage } from "../core-bridge/types";
import { utf8ByteLength } from "./snapshotUpload";

export const TRANSCRIPT_PAGE_ITEMS = 80;
export const TRANSCRIPT_TAIL_ITEMS = 160;
export const TRANSCRIPT_PAGE_TARGET_BYTES = 6 * 1024 * 1024;
// A page normally seals around 6 MiB. The larger singleton ceiling covers the
// product's bounded inline records (including base64 expansion) without making
// ordinary reads or writes large.
export const TRANSCRIPT_PAGE_HARD_BYTES = 32 * 1024 * 1024;
export const TRANSCRIPT_BATCH_TARGET_BYTES = 24 * 1024 * 1024;

export interface PagedSnapshotUpload {
  head: Snapshot;
  sealedThrough: number;
  pageStartLocal: number;
  pageEndLocal: number;
}

function referencedPage(snapshot: Snapshot, startIndex: number, items: TimelineItem[]): TranscriptPage {
  const toolCalls: Snapshot["tool_calls"] = {};
  const artifacts: Snapshot["artifacts"] = [];
  const providerIncidents: Snapshot["provider_incidents"] = {};
  for (const item of items) {
    if (item.item === "tool_call" && snapshot.tool_calls[item.id]) {
      toolCalls[item.id] = snapshot.tool_calls[item.id];
    } else if (item.item === "artifact") {
      const artifact = snapshot.artifacts.find((entry) => entry.id === item.id);
      if (artifact) artifacts.push(artifact);
    } else if (item.item === "provider_incident" && snapshot.provider_incidents[item.id]) {
      providerIncidents[item.id] = snapshot.provider_incidents[item.id];
    }
  }
  return {
    startIndex,
    items,
    ...(Object.keys(toolCalls).length > 0 ? { toolCalls } : {}),
    ...(artifacts.length > 0 ? { artifacts } : {}),
    ...(Object.keys(providerIncidents).length > 0 ? { providerIncidents } : {}),
  };
}

function pageWireBytes(page: TranscriptPage): number {
  return utf8ByteLength(JSON.stringify(page));
}

export function* transcriptPageBatches(
  snapshot: Snapshot,
  localStart: number,
  localEnd: number,
): Generator<TranscriptPage[]> {
  const base = snapshot.timeline_offset ?? 0;
  let batch: TranscriptPage[] = [];
  let batchBytes = 0;
  let cursor = localStart;
  while (cursor < localEnd) {
    let count = 0;
    let estimatedBytes = 0;
    while (count < TRANSCRIPT_PAGE_ITEMS && cursor + count < localEnd) {
      const item = snapshot.timeline[cursor + count];
      const itemBytes = pageWireBytes(referencedPage(snapshot, base + cursor + count, [item]));
      if (count > 0 && estimatedBytes + itemBytes > TRANSCRIPT_PAGE_TARGET_BYTES) break;
      estimatedBytes += itemBytes;
      count += 1;
    }
    const page = referencedPage(snapshot, base + cursor, snapshot.timeline.slice(cursor, cursor + count));
    const bytes = pageWireBytes(page);
    if (bytes > TRANSCRIPT_PAGE_HARD_BYTES) {
      throw new Error("one transcript record exceeds the immutable page boundary");
    }
    if (batch.length > 0 && batchBytes + bytes > TRANSCRIPT_BATCH_TARGET_BYTES) {
      yield batch;
      batch = [];
      batchBytes = 0;
    }
    batch.push(page);
    batchBytes += bytes;
    cursor += count;
    if (batch.length === 4) {
      yield batch;
      batch = [];
      batchBytes = 0;
    }
  }
  if (batch.length > 0) yield batch;
}

function active(snapshot: Snapshot): boolean {
  return Boolean(snapshot.starting) || Object.values(snapshot.runs).some((run) => (
    run.status === "queued" || run.status === "running" || run.status === "awaiting_input"
  ));
}

/**
 * Produce only the immutable delta after `alreadySealedThrough` plus a small
 * live head. No full-snapshot stringify or clone occurs: arrays are sliced and
 * each page is encoded independently at the native boundary.
 */
export function preparePagedSnapshot(
  snapshot: Snapshot,
  alreadySealedThrough: number,
): PagedSnapshotUpload {
  const base = snapshot.timeline_offset ?? 0;
  const localStart = alreadySealedThrough - base;
  const checkpoint = snapshot.model_context_checkpoint?.timeline_index;
  if (
    active(snapshot)
    || checkpoint === undefined
    || localStart < 0
    || localStart > snapshot.timeline.length
  ) {
    return {
      head: snapshot,
      sealedThrough: base,
      pageStartLocal: 0,
      pageEndLocal: 0,
    };
  }
  const compactedEnd = Math.min(snapshot.timeline.length, Math.max(0, checkpoint - base));
  const sealEnd = Math.max(localStart, compactedEnd - TRANSCRIPT_TAIL_ITEMS);
  if (sealEnd <= localStart) {
    return {
      head: snapshot,
      sealedThrough: alreadySealedThrough,
      pageStartLocal: localStart,
      pageEndLocal: localStart,
    };
  }
  const sealedThrough = base + sealEnd;
  const timeline = snapshot.timeline.slice(sealEnd);
  const toolIds = new Set(timeline.flatMap((item) => item.item === "tool_call" ? [item.id] : []));
  const artifactIds = new Set(timeline.flatMap((item) => item.item === "artifact" ? [item.id] : []));
  const incidentIds = new Set(
    timeline.flatMap((item) => item.item === "provider_incident" ? [item.id] : []),
  );
  return {
    sealedThrough,
    pageStartLocal: localStart,
    pageEndLocal: sealEnd,
    head: {
      ...snapshot,
      timeline_offset: sealedThrough,
      timeline,
      tool_calls: Object.fromEntries(
        Object.entries(snapshot.tool_calls).filter(([id]) => toolIds.has(id)),
      ),
      artifacts: snapshot.artifacts.filter((artifact) => artifactIds.has(artifact.id)),
      provider_incidents: Object.fromEntries(
        Object.entries(snapshot.provider_incidents).filter(([id]) => incidentIds.has(id)),
      ),
    },
  };
}
