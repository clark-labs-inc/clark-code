import type { Snapshot } from "../core-bridge/types";

/**
 * Whether a snapshot deferred off the render path still represents the live
 * projection. Snapshot objects are immutable emissions from the native bridge,
 * so identity is a generation token: any newer event replaces `latest`.
 */
export function deferredSnapshotPersistIsCurrent(
  latest: Snapshot,
  candidate: Snapshot,
): boolean {
  return latest === candidate;
}
