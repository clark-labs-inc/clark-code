// Dev/test bridge: talks to the `devbridge` Rust server, which runs the REAL
// providers + agent-core projection. Lets the browser drive real Clark/ACP turns
// (for headless UI testing and video capture) with zero logic duplicated in TS —
// it only relays commands and renders the Snapshots the engine produces.

import type { CoreBridge, ConnectConfig, SessionOptions } from "./bridge";
import type { Upload } from "../lib/attachments";
import {
  emptySnapshot,
  type ClientResponse,
  type ContentBlock,
  type ProviderInfo,
  type Session,
  type Snapshot,
} from "./types";

export class DevBridge implements CoreBridge {
  private ws: WebSocket;
  private ready: Promise<void>;
  private pending = new Map<number, (msg: Record<string, unknown>) => void>();
  private nextId = 1;
  private handlers = new Set<(s: Snapshot) => void>();
  private snapshot: Snapshot = emptySnapshot();

  constructor(url = "ws://localhost:7878") {
    this.ws = new WebSocket(url);
    this.ready = new Promise((resolve, reject) => {
      this.ws.onopen = () => resolve();
      this.ws.onerror = () => reject(new Error(`devbridge unreachable at ${url}`));
    });
    this.ws.onmessage = (event) => {
      const msg = JSON.parse(event.data as string) as Record<string, unknown>;
      if (msg.type === "snapshot") {
        this.snapshot = msg.snapshot as Snapshot;
        for (const h of this.handlers) h(this.snapshot);
        return;
      }
      const id = msg.id as number | undefined;
      if (id != null && this.pending.has(id)) {
        this.pending.get(id)!(msg);
        this.pending.delete(id);
      }
    };
  }

  private async call(cmd: Record<string, unknown>): Promise<Record<string, unknown>> {
    await this.ready;
    const id = this.nextId++;
    return new Promise((resolve) => {
      this.pending.set(id, resolve);
      this.ws.send(JSON.stringify({ id, ...cmd }));
    });
  }

  private async fire(cmd: Record<string, unknown>): Promise<void> {
    await this.ready;
    this.ws.send(JSON.stringify(cmd));
  }

  async listProviders(): Promise<ProviderInfo[]> {
    const r = await this.call({ cmd: "list_providers" });
    return r.providers as ProviderInfo[];
  }

  async connect(providerId: string, config: ConnectConfig): Promise<void> {
    const r = await this.call({ cmd: "connect", provider: providerId, config });
    if (r.type === "error") throw new Error(String(r.message));
  }

  async newSession(providerId: string, options: SessionOptions): Promise<Session> {
    const r = await this.call({ cmd: "new_session", provider: providerId, options });
    if (r.type === "error") throw new Error(String(r.message));
    return r.session as Session;
  }

  async prompt(
    sessionId: string,
    blocks: ContentBlock[],
    attachments: Upload[] = [],
  ): Promise<void> {
    await this.fire({ cmd: "prompt", session: sessionId, blocks, attachments });
  }

  async cancel(sessionId: string, runId: string): Promise<void> {
    await this.fire({ cmd: "cancel", session: sessionId, run: runId });
  }

  async respond(sessionId: string, response: ClientResponse): Promise<void> {
    await this.fire({ cmd: "respond", session: sessionId, response });
  }

  subscribe(handler: (s: Snapshot) => void): () => void {
    this.handlers.add(handler);
    handler(this.snapshot);
    return () => this.handlers.delete(handler);
  }
}
