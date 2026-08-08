// Read-only SSH host discovery. Remote coding workers are owned separately by
// the native runtime registry (`remoteWorker.ts`).

import { invoke } from "@tauri-apps/api/core";

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Result of a read-only "test connection". */
export interface SshProbe {
  /** Detected architecture slug (e.g. `linux-x86_64`). */
  arch: string;
  /** The remote `$HOME`. */
  home: string;
}

/** Reach a host and report arch + home (no deploy/tunnel). Throws on failure
 *  (unreachable, unsupported arch) with a readable message. */
export function probeSsh(host: string): Promise<SshProbe> {
  if (!isTauri()) return Promise.reject(new Error("SSH testing is available in the desktop app."));
  return invoke<SshProbe>("ssh_probe", { host });
}

export interface RemoteDirectory {
  name: string;
  path: string;
}

export interface RemoteDirectoryListing {
  path: string;
  parent: string | null;
  directories: RemoteDirectory[];
}

/** List folders on an SSH host without deploying or starting an agent. */
export function listSshDirectories(
  host: string,
  path?: string | null,
): Promise<RemoteDirectoryListing> {
  if (!isTauri()) {
    return Promise.reject(new Error("Remote folder browsing is available in the desktop app."));
  }
  return invoke<RemoteDirectoryListing>("ssh_list_directories", {
    host,
    path: path?.trim() || null,
  });
}
