// The single seam between the UI and `agent-core`.
//
// Surfaces only ever call this interface; they never know whether the engine is
// running native in the Tauri host (production) or as a mock (browser preview /
// tests). A future WASM build of agent-core slots in as a third implementation
// behind the same interface.

import type {
  ClientResponse,
  ProviderInfo,
  Session,
  Snapshot,
  ContentBlock,
  MemoryOverview,
} from "./types";
import type { Upload } from "../lib/attachments";

export interface ConnectConfig {
  endpoint?: string;
  command?: string[];
  cwd?: string;
  auth_token?: string;
  /** Provider-specific extras (e.g. local coding: base_url, model, clark). */
  extra?: Record<string, unknown>;
  headers?: Record<string, string>;
}

export interface SessionOptions {
  cwd?: string;
  mode?: string;
}

export interface CoreBridge {
  listProviders(): Promise<ProviderInfo[]>;
  connect(providerId: string, config: ConnectConfig): Promise<void>;
  newSession(providerId: string, options: SessionOptions): Promise<Session>;
  /** Resume a prior session by id (capability-gated: `load_session`). */
  loadSession(providerId: string, id: string): Promise<Session>;
  prompt(sessionId: string, blocks: ContentBlock[], attachments?: Upload[]): Promise<void>;
  cancel(sessionId: string, runId: string): Promise<void>;
  respond(sessionId: string, response: ClientResponse): Promise<void>;
  /** Subscribe to snapshot updates. Returns an unsubscribe fn. */
  subscribe(handler: (snapshot: Snapshot) => void): () => void;
  /**
   * Extract a per-repo project memory via Clark's Platform API and write it
   * under the repo's `.clark/memory/`. Only the native (Tauri) bridge supports
   * this.
   */
  extractMemory?(cwd: string, apiKey: string, model?: string): Promise<string>;
  /**
   * List the per-repo memory (the `MEMORY.md` index plus any per-fact files)
   * under `<cwd>/.clark/memory/`. Read-only. Only the native (Tauri) bridge
   * supports this.
   */
  listMemory?(cwd: string): Promise<MemoryOverview>;
  /** Project-relative file paths under `cwd`, for the `@`-mention picker. */
  listFiles?(cwd: string): Promise<string[]>;
  /** Open a path in the OS default app, or reveal it in the file manager. */
  openPath?(path: string, reveal?: boolean): Promise<void>;
}

let cached: CoreBridge | null = null;

/**
 * Returns the right bridge:
 * - the Tauri-backed bridge inside the desktop app;
 * - the DevBridge (real providers via the `devbridge` server) when the page is
 *   loaded with `?dev` — used for headless real-Clark testing and video;
 * - the mock otherwise (plain browser preview).
 */
export async function getBridge(): Promise<CoreBridge> {
  if (cached) return cached;
  const runningInTauri =
    typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  const params =
    typeof window !== "undefined" ? new URLSearchParams(window.location.search) : null;

  if (runningInTauri) {
    const { TauriBridge } = await import("./tauriBridge");
    cached = new TauriBridge();
  } else if (params?.has("dev")) {
    const { DevBridge } = await import("./devBridge");
    cached = new DevBridge(params.get("dev") || undefined);
  } else {
    const { MockBridge } = await import("./mockBridge");
    cached = new MockBridge();
  }
  return cached;
}
