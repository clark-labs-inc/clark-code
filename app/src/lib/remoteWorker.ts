// Native-owned durable remote coding worker. The renderer chooses only the
// saved SSH destination, project root, and catalog model. Native code resolves
// the signed binary, app-encrypted credential, worker policy, and account
// partition. No operating-system credential store participates.

import { invoke } from "@tauri-apps/api/core";

export interface RemoteInfo {
  /** Opaque, account-bound native worker capability. */
  id: string;
  cwd: string;
  arch: string;
  sshTransport: "control_master";
  connectionKind: "started" | "reused" | "replaced";
  connectDurationMs: number;
  accountWorkerCount: number;
}

export interface RemoteTargetConfig {
  worker_handle: string;
  cwd: string;
}

const connecting = new Map<string, Promise<RemoteInfo>>();

export function remoteWorkerConnect(
  host: string,
  remoteRoot: string,
  model: string,
  reasoningEffort: string,
): Promise<RemoteInfo> {
  const input = { host, remoteRoot, model, reasoningEffort };
  const key = JSON.stringify(input);
  const current = connecting.get(key);
  if (current) return current;
  const request = invoke<RemoteInfo>("remote_worker_connect", { input }).finally(() =>
    connecting.delete(key),
  );
  connecting.set(key, request);
  return request;
}

export function remoteTarget(info: RemoteInfo): RemoteTargetConfig {
  return { worker_handle: info.id, cwd: info.cwd };
}
