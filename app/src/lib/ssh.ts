// Client for the SSH orchestration Tauri commands. `sshConnect` brings up a
// clark-exec-server on the remote + a loopback tunnel and returns the ws URL +
// capability token the local provider connects through; `sshDisconnect` tears
// it down. Desktop-only (no-op shapes in the browser preview).

import { invoke } from "@tauri-apps/api/core";

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** What the host returns after a remote project connects. */
export interface RemoteInfo {
  /** Handle for `sshDisconnect`. */
  id: string;
  /** `ws://127.0.0.1:<port>` — the local end of the SSH tunnel. */
  ws_url: string;
  /** Per-session capability token the exec-server checks. */
  token: string;
  /** Absolute project root on the remote. */
  cwd: string;
  /** Detected remote architecture (e.g. `linux-x86_64`). */
  arch: string;
}

/** The `remote` block spread into the local provider's connect `extra`. */
export interface RemoteTargetConfig {
  ws_url: string;
  token: string;
  cwd: string;
}

/** Bring up the remote server + tunnel. The server is fetched from the CDN for
 *  the remote's arch; `localBinary` is an optional dev override. Throws with a
 *  readable message on failure (host unreachable, arch mismatch, …). */
export function sshConnect(
  host: string,
  remoteRoot: string,
  localBinary?: string,
): Promise<RemoteInfo> {
  return invoke<RemoteInfo>("ssh_connect", {
    host,
    remoteRoot,
    localBinary: localBinary?.trim() || null,
  });
}

/** Tear down a remote connection by id. Best-effort; never throws in practice. */
export async function sshDisconnect(id: string): Promise<void> {
  if (!isTauri() || !id) return;
  try {
    await invoke("ssh_disconnect", { id });
  } catch {
    /* already gone — fine */
  }
}

/** The connect `extra.remote` shape the provider expects. */
export function remoteTarget(info: RemoteInfo): RemoteTargetConfig {
  return { ws_url: info.ws_url, token: info.token, cwd: info.cwd };
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
