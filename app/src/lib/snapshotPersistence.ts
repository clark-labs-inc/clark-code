import type { Snapshot } from "../core-bridge/types";

/** Coalesce newer active deltas into a deferred checkpoint, while forbidding
 * an older busy callback from persisting after the run became terminal. */
export function deferredSnapshotForPersistence(
  latest: Snapshot,
  candidate: Snapshot,
  wasBusy: boolean,
  stillBusy: boolean,
): Snapshot | null {
  if (latest.session !== candidate.session) return null;
  if (wasBusy) return stillBusy ? latest : null;
  return latest === candidate ? candidate : null;
}
