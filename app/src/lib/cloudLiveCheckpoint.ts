import type { Snapshot } from "../core-bridge/types";
import { historyFingerprintHead } from "./historyFingerprint";

function exceedsApproximateChars(value: unknown, limit: number): boolean {
  const stack: unknown[] = [value];
  let total = 0;
  while (stack.length > 0) {
    const current = stack.pop();
    if (typeof current === "string") total += current.length;
    else if (Array.isArray(current)) stack.push(...current);
    else if (current && typeof current === "object") stack.push(...Object.values(current));
    if (total > limit) return true;
  }
  return false;
}

/** The cursor is an observation of native/cloud acknowledgement, never a
 * renderer-selected publication boundary. Native revalidates the exact head. */
export class CloudLiveCheckpointBounds {
  private sealedThrough = new Map<string, number>();
  private assignedRevision = new Map<string, number>();

  record(id: string, cursor: number | undefined): void {
    if (cursor !== undefined && Number.isSafeInteger(cursor) && cursor >= 0) {
      this.sealedThrough.set(id, Math.max(this.sealedThrough.get(id) ?? 0, cursor));
    }
  }
  clear(): void { this.sealedThrough.clear(); this.assignedRevision.clear(); }
  forget(id: string): void { this.sealedThrough.delete(id); this.assignedRevision.delete(id); }
  nextRevision(id: string, nowMs: number, acknowledged: number): number {
    const rev = Math.max(nowMs, acknowledged + 1, (this.assignedRevision.get(id) ?? 0) + 1);
    this.assignedRevision.set(id, rev);
    return rev;
  }
  head(id: string, snapshot: Snapshot): Snapshot {
    return historyFingerprintHead(snapshot, this.sealedThrough.get(id) ?? snapshot.timeline_offset ?? 0);
  }
  deferLive(id: string, snapshot: Snapshot): boolean {
    const head = this.head(id, snapshot);
    // Count payload, not turns. Hundreds of small turns must still stream;
    // genuinely oversized unsealed records retain the bounded idle backstop.
    return exceedsApproximateChars(head, 1024 * 1024);
  }
}
