import type { Snapshot } from "../core-bridge/types";

export interface DrainableSession {
  live: Snapshot;
  queuedCount: number;
  dispatching: boolean;
  starting: boolean;
}

/** True while one session still owns work that must finish before installation.
 *  Permission waits are included explicitly even though current local runs stay
 *  `running` while gated; this keeps the contract correct for other providers. */
export function sessionBlocksUpdate(session: DrainableSession): boolean {
  const activeRun = Object.values(session.live.runs).some(
    (run) =>
      run.status === "running" ||
      run.status === "queued" ||
      run.status === "awaiting_input",
  );
  return (
    activeRun ||
    !!session.live.pending_permission ||
    session.queuedCount > 0 ||
    session.dispatching ||
    session.starting
  );
}

/** Count blocked conversations, not individual runs, for stable user messaging. */
export function updateDrainBlockerCount(sessions: Iterable<DrainableSession>): number {
  let count = 0;
  for (const session of sessions) {
    if (sessionBlocksUpdate(session)) count += 1;
  }
  return count;
}
