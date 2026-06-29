// Production bridge: thin wrapper over Tauri commands + events. The heavy
// lifting (transport, projection) happens in the native `agent-core` host.
//
// The matching Rust commands are registered in src-tauri.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { CoreBridge, ConnectConfig, SessionOptions } from "./bridge";
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

  newSession(providerId: string, options: SessionOptions): Promise<Session> {
    return invoke<Session>("session_new", { providerId, options });
  }

  loadSession(providerId: string, id: string): Promise<Session> {
    return invoke<Session>("session_load", { providerId, id });
  }

  prompt(sessionId: string, blocks: ContentBlock[], attachments: Upload[] = []): Promise<void> {
    return invoke("prompt", { sessionId, blocks, attachments });
  }

  cancel(sessionId: string, runId: string): Promise<void> {
    return invoke("cancel", { sessionId, runId });
  }

  respond(sessionId: string, response: ClientResponse): Promise<void> {
    return invoke("respond", { sessionId, response });
  }

  subscribe(handler: (snapshot: Snapshot) => void): () => void {
    const unlisten = listen<Snapshot>("snapshot", (event) => {
      handler(event.payload);
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }

  extractMemory(cwd: string, apiKey: string, model?: string): Promise<string> {
    return invoke<string>("local_extract_memory", { cwd, apiKey, model });
  }

  listMemory(cwd: string): Promise<MemoryOverview> {
    return invoke<MemoryOverview>("local_list_memory", { cwd });
  }

  listFiles(cwd: string): Promise<string[]> {
    return invoke<string[]>("local_list_files", { cwd });
  }

  openPath(path: string, reveal = false): Promise<void> {
    return invoke("open_path", { path, reveal });
  }
}
