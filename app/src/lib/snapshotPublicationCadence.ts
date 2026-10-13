import type { Snapshot } from "../core-bridge/types";
import { parseNarration } from "./narration";

export const STREAMING_PUBLICATION_INTERVAL_MS = 250;

function latestPublicMessageKey(snapshot: Snapshot): string | null {
  for (let index = snapshot.timeline.length - 1; index >= 0; index -= 1) {
    const item = snapshot.timeline[index];
    if (item.item !== "message" || item.role !== "agent") continue;
    const text = item.blocks.flatMap((block) => block.type === "text" ? [block.text] : []).join("\n");
    const visible = parseNarration(text).some((span) => span.kind !== "thinking" && span.text.trim());
    return visible ? `${item.run}:${(snapshot.timeline_offset ?? 0) + index}` : null;
  }
  return null;
}

export class SnapshotPublicationCadence {
  private lastPublishedAt = -Infinity;
  private publicMessageKey: string | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private pending: { snapshot: Snapshot; busy: boolean; publish: () => void } | null = null;

  shouldPublish(snapshot: Snapshot, busy: boolean, nowMs: number): boolean {
    const key = latestPublicMessageKey(snapshot);
    const firstPublicText = key !== null && key !== this.publicMessageKey;
    if (busy && !firstPublicText && nowMs - this.lastPublishedAt < STREAMING_PUBLICATION_INTERVAL_MS) return false;
    this.lastPublishedAt = nowMs;
    this.publicMessageKey = key;
    return true;
  }

  schedule(snapshot: Snapshot, busy: boolean, publish: () => void): void {
    if (this.shouldPublish(snapshot, busy, Date.now())) {
      if (this.timer !== null) clearTimeout(this.timer);
      this.timer = null;
      this.pending = null;
      publish();
      return;
    }
    this.pending = { snapshot, busy, publish };
    if (this.timer !== null) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      const pending = this.pending;
      this.pending = null;
      if (pending) this.schedule(pending.snapshot, pending.busy, pending.publish);
    }, Math.max(0, STREAMING_PUBLICATION_INTERVAL_MS - (Date.now() - this.lastPublishedAt)));
  }
}
