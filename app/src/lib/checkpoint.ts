// Client for the working-tree checkpoint / undo Tauri command.

import { invoke } from "@tauri-apps/api/core";
import type { RemoteInfo } from "./ssh";

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Restore the project at `cwd` to a pre-run checkpoint. Throws with a
 *  user-facing message (e.g. "Undo needs a git repository."). */
export async function restoreCheckpoint(
  cwd: string,
  sha: string,
  remoteInfo?: RemoteInfo | null,
): Promise<void> {
  if (!isTauri()) throw new Error("Undo is available in the desktop app.");
  const remote = remoteInfo
    ? { ws_url: remoteInfo.ws_url, token: remoteInfo.token }
    : null;
  await invoke("clark_checkpoint_restore", { cwd, sha, remote });
}
