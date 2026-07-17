// Production bridge: thin wrapper over Tauri commands + events. The heavy
// lifting (transport, projection) happens in the native `agent-core` host.
//
// The matching Rust commands are registered in src-tauri.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  CloudTrajectoryConfig,
  CoreBridge,
  ConnectConfig,
  SessionOptions,
} from "./bridge";
import type { Upload } from "../lib/attachments";
import type {
  ClientResponse,
  ContentBlock,
  ProviderInfo,
  Session,
  Snapshot,
  MemoryOverview,
} from "./types";

export class TauriBridge implements CoreBridge {
  listProviders(): Promise<ProviderInfo[]> {
    return invoke<ProviderInfo[]>("provider_list");
  }

  connect(providerId: string, config: ConnectConfig): Promise<void> {
    return invoke("provider_connect", { providerId, config });
  }

  reconfigure(sessionId: string, config: ConnectConfig): Promise<void> {
    return invoke("provider_reconfigure", { sessionId, config });
  }

  newSession(providerId: string, options: SessionOptions, bindId?: string): Promise<Session> {
    return invoke<Session>("session_new", { providerId, options, bindId: bindId ?? null });
  }

  loadSession(providerId: string, id: string): Promise<Session> {
    return invoke<Session>("session_load", { providerId, id });
  }

  closeSession(sessionId: string): Promise<void> {
    return invoke("session_close", { sessionId });
  }

  configureCloudTrajectory(sessionId: string, config: CloudTrajectoryConfig): Promise<void> {
    return invoke("session_configure_cloud", { sessionId, config });
  }

  updateCloudToken(token: string): Promise<void> {
    return invoke("update_cloud_token", { token });
  }

  onCloudAuthExpired(handler: () => void): () => void {
    const unlisten = listen("cloud-auth-expired", () => handler());
    return () => {
      void unlisten.then((fn) => fn());
    };
  }

  onCloudSyncWarning(handler: (message: string) => void): () => void {
    const unlisten = listen<string>("cloud-sync-warning", (event) => handler(event.payload));
    return () => {
      void unlisten.then((fn) => fn());
    };
  }

  prompt(sessionId: string, blocks: ContentBlock[], attachments: Upload[] = []): Promise<void> {
    return invoke("prompt", { sessionId, blocks, attachments });
  }

  steer(sessionId: string, blocks: ContentBlock[]): Promise<void> {
    return invoke("steer", { sessionId, blocks });
  }

  cancel(sessionId: string, runId: string): Promise<void> {
    return invoke("cancel", { sessionId, runId });
  }

  respond(sessionId: string, response: ClientResponse): Promise<void> {
    return invoke("respond", { sessionId, response });
  }

  setMode(sessionId: string, mode: string): Promise<void> {
    return invoke("set_mode", { sessionId, mode });
  }

  setOutputStyle(sessionId: string, style: string): Promise<void> {
    return invoke("set_output_style", { sessionId, style });
  }

  subscribe(handler: (snapshot: Snapshot) => void): () => void {
    const unlisten = listen<Snapshot>("snapshot", (event) => {
      handler(event.payload);
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }

  listMemory(cwd: string, remote?: { ws_url: string; token: string } | null): Promise<MemoryOverview> {
    return invoke<MemoryOverview>("local_list_memory", { cwd, remote: remote ?? null });
  }

  listGlobalMemory(): Promise<MemoryOverview> {
    return invoke<MemoryOverview>("local_list_global_memory");
  }

  listFiles(cwd: string, remote?: { ws_url: string; token: string } | null): Promise<string[]> {
    return invoke<string[]>("local_list_files", { cwd, remote: remote ?? null });
  }

  openPath(path: string, reveal = false): Promise<void> {
    return invoke("open_path", { path, reveal });
  }
}
