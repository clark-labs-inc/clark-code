// Serialize conversation snapshots exactly before cloud sync. Size policy is
// enforced by the caller as an explicit failed/queued sync boundary; this
// module never rewrites accepted history to fit that boundary.

import type { Snapshot } from "../core-bridge/types";

export interface PreparedSnapshot {
  /** The exact snapshot to upload. */
  snapshot: Snapshot;
  /** Serialized once for fingerprinting and the caller's hard-cap check. */
  json: string;
  /** UTF-8 byte length of `json`, matching HTTP body accounting. */
  bytes: number;
}

/** Serialized request bodies are UTF-8; JavaScript string length counts UTF-16
 * code units and can undercount non-ASCII history by almost 2×. */
export function utf8ByteLength(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

/** Prepare a snapshot for byte-for-byte cloud upload. */
export function prepareSnapshotForUpload(snapshot: Snapshot): PreparedSnapshot {
  const json = JSON.stringify(snapshot);
  const bytes = utf8ByteLength(json);
  return { snapshot, json, bytes };
}
