import { invoke } from "@tauri-apps/api/core";
import type { Snapshot } from "../core-bridge/types";
import type { RemoteInfo } from "./ssh";

type RemoteExecutorArg = Pick<RemoteInfo, "ws_url" | "token">;

export function snapshotCheckpointIds(snapshot: Snapshot): string[] {
  return [...new Set(
    Object.values(snapshot.runs)
      .map((run) => run.checkpoint)
      .filter((checkpoint): checkpoint is string => !!checkpoint),
  )];
}

export async function releaseSnapshotCheckpoints(
  cwd: string,
  snapshot: Snapshot,
  remote: RemoteExecutorArg | null,
): Promise<void> {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return;
  const checkpoints = snapshotCheckpointIds(snapshot);
  if (!cwd || checkpoints.length === 0) return;
  await invoke("changes_release_checkpoints", { cwd, checkpoints, remote });
}
