// Dev/test bridge: talks to the `devbridge` Rust server, which runs the REAL
// providers + agent-core projection. Lets the browser drive real the agent/ACP turns
// (for headless UI testing and video capture) with zero logic duplicated in TS —
// it only relays commands and renders the Snapshots the engine produces.

import type {
  CoreBridge,
  ConnectConfig,
  PromptReceipt,
} from "./bridge";
import type { Upload } from "../lib/attachments";
import {
  emptySnapshot,
  normalizeSnapshot,
  type ClientResponse,
  type ContentBlock,
  type ProviderInfo,
  type Session,
  type Snapshot,
  type WireSnapshot,
} from "./types";

export class DevBridge implements CoreBridge {
  private ws: WebSocket;
  private ready: Promise<void>;
  private pending = new Map<number, (msg: Record<string, unknown>) => void>();
  private nextId = 1;
  private handlers = new Set<(s: Snapshot) => void>();
  private snapshot: Snapshot = emptySnapshot();
  /** devbridge is single-session and tags snapshots with the provider's own
   *  session id; when `openSession` rebinds to a conversation id, rewrite the
   *  tag so the store can route by it. */
  private alias: { from: string; to: string } | null = null;

  constructor(url = "ws://localhost:7878") {
    this.ws = new WebSocket(url);
    this.ready = new Promise((resolve, reject) => {
      this.ws.onopen = () => resolve();
      this.ws.onerror = () => reject(new Error(`devbridge unreachable at ${url}`));
    });
    this.ws.onmessage = (event) => {
      const msg = JSON.parse(event.data as string) as Record<string, unknown>;
      if (msg.type === "snapshot") {
        const snap = normalizeSnapshot(msg.snapshot as WireSnapshot);
        if (this.alias && snap.session === this.alias.from) snap.session = this.alias.to;
        this.snapshot = snap;
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

  async openSession(
    providerId: string,
    config: ConnectConfig,
    request: import("./bridge").SessionOpenRequest,
  ): Promise<Session> {
    const r = await this.call({ cmd: "open_session", provider: providerId, config, request });
    if (r.type === "error") throw new Error(String(r.message));
    const session = r.session as Session;
    if (request.kind !== "new" || !request.bindId) return session;
    this.alias = { from: session.id, to: request.bindId };
    return { ...session, id: request.bindId };
  }

  async prompt(
    sessionId: string,
    blocks: ContentBlock[],
    attachments: Upload[] = [],
  ): Promise<PromptReceipt> {
    const response = await this.call({
      cmd: "prompt",
      session: sessionId,
      blocks,
      attachments,
    });
    if (response.type === "error") throw new Error(String(response.message));
    return { runId: String(response.runId) };
  }

  async cancel(sessionId: string, runId: string): Promise<void> {
    await this.fire({ cmd: "cancel", session: sessionId, run: runId });
  }

  async respond(sessionId: string, response: ClientResponse): Promise<void> {
    await this.fire({ cmd: "respond", session: sessionId, response });
  }

  // devbridge's WS protocol doesn't implement the `/btw` fork (it's a dev-only
  // transport, not the full Tauri command surface). Reject so the overlay
  // shows a clean error instead of hanging; use the native app or the mock
  // preview to exercise this feature.
  async sideQuestion(_sessionId: string, _question: string): Promise<string> {
    throw new Error("side_question is not implemented in devbridge");
  }

  subscribe(handler: (s: Snapshot) => void): () => void {
    this.handlers.add(handler);
    handler(this.snapshot);
    return () => this.handlers.delete(handler);
  }
}
