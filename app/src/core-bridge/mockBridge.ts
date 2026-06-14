// Test-double bridge used in a plain browser (vite dev / Vitest). It emits
// pre-baked Snapshots to simulate a streaming agent run, so the UI is fully
// demonstrable without the native host. It deliberately does NOT re-implement
// the reducer — it just produces snapshots a real run would yield.

import type { CoreBridge, ConnectConfig, SessionOptions } from "./bridge";
import {
  emptySnapshot,
  type ClientResponse,
  type ContentBlock,
  type ProviderInfo,
  type Session,
  type Snapshot,
} from "./types";

const PROVIDERS: ProviderInfo[] = [
  {
    id: "acp",
    label: "ACP (local CLI agent)",
    capabilities: {
      streaming: true,
      permissions: true,
      fs: true,
      terminal: true,
      load_session: true,
      modes: ["default", "plan"],
    },
  },
  {
    id: "clark",
    label: "Clark",
    capabilities: {
      streaming: true,
      permissions: true,
      fs: true,
      terminal: true,
      load_session: true,
      modes: ["clark", "clark_max"],
    },
  },
];

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export class MockBridge implements CoreBridge {
  private snapshot: Snapshot = emptySnapshot();
  private handlers = new Set<(s: Snapshot) => void>();

  async listProviders(): Promise<ProviderInfo[]> {
    return PROVIDERS;
  }

  async connect(_providerId: string, _config: ConnectConfig): Promise<void> {}

  async newSession(providerId: string, _options: SessionOptions): Promise<Session> {
    const provider = PROVIDERS.find((p) => p.id === providerId) ?? PROVIDERS[0];
    this.snapshot = { ...emptySnapshot(), session: "mock-session" };
    this.emit();
    return {
      id: "mock-session",
      provider: provider.id,
      capabilities: provider.capabilities,
      mode: provider.capabilities.modes[0],
    };
  }

  async loadSession(providerId: string, id: string): Promise<Session> {
    // The mock has no server state; the store restores the persisted snapshot.
    const provider = PROVIDERS.find((p) => p.id === providerId) ?? PROVIDERS[0];
    this.snapshot = { ...emptySnapshot(), session: id };
    return {
      id,
      provider: provider.id,
      capabilities: provider.capabilities,
      mode: provider.capabilities.modes[0],
    };
  }

  async prompt(
    _sessionId: string,
    blocks: ContentBlock[],
    _attachments: import("../lib/attachments").Upload[] = [],
  ): Promise<void> {
    const userText = blocks
      .map((b) => (b.type === "text" ? b.text : "[attachment]"))
      .join(" ");
    void this.playRun(userText);
  }

  async cancel(): Promise<void> {
    const last = this.lastRunId();
    if (last && this.snapshot.runs[last]) {
      this.snapshot.runs[last] = { id: last, status: "cancelled" };
      this.emit();
    }
  }

  async respond(_sessionId: string, response: ClientResponse): Promise<void> {
    if (response.kind === "permission") {
      this.snapshot = { ...this.snapshot, pending_permission: undefined };
      this.emit();
    }
  }

  subscribe(handler: (s: Snapshot) => void): () => void {
    this.handlers.add(handler);
    handler(this.snapshot);
    return () => this.handlers.delete(handler);
  }

  // --- internals -----------------------------------------------------------

  private lastRunId(): string | undefined {
    const ids = Object.keys(this.snapshot.runs);
    return ids[ids.length - 1];
  }

  private emit() {
    const frozen = structuredClone(this.snapshot);
    for (const h of this.handlers) h(frozen);
  }

  /** Simulate a realistic streaming run: user turn → plan → tool call →
   *  permission gate → streamed answer → done. */
  private async playRun(userText: string) {
    const run = `run-${Date.now()}`;
    this.snapshot.runs[run] = { id: run, status: "running" };
    this.snapshot.timeline.push({
      item: "message",
      run,
      role: "user",
      blocks: [{ type: "text", text: userText }],
    });
    this.emit();
    await sleep(250);

    this.snapshot.plan = {
      phases: [
        { title: "Inspect the workspace", status: "in_progress" },
        { title: "Apply the change", status: "pending" },
      ],
    };
    if (!this.snapshot.timeline.some((t) => t.item === "plan")) {
      this.snapshot.timeline.push({ item: "plan" });
    }
    this.emit();
    await sleep(300);

    const tc = `tc-${Date.now()}`;
    this.snapshot.tool_calls[tc] = {
      id: tc,
      title: "Read src/main.rs",
      kind: "read",
      status: "in_progress",
      locations: [{ path: "src/main.rs", line: 1 }],
      content: [],
    };
    this.snapshot.timeline.push({ item: "tool_call", id: tc });
    this.snapshot.focus = { surface: "files", path: "src/main.rs" };
    this.emit();
    await sleep(400);

    this.snapshot.tool_calls[tc] = {
      ...this.snapshot.tool_calls[tc],
      status: "completed",
      content: [{ type: "text", text: "fn main() { println!(\"hello\"); }" }],
    };
    this.emit();
    await sleep(250);

    // An edit tool call produces a diff in the Files surface.
    const edit = `tc-edit-${Date.now()}`;
    this.snapshot.tool_calls[edit] = {
      id: edit,
      title: "Edit src/main.rs",
      kind: "edit",
      status: "completed",
      locations: [{ path: "src/main.rs", line: 1 }],
      content: [
        {
          type: "text",
          text:
            "diff src/main.rs\n" +
            '-fn main() { println!("hello"); }\n' +
            "+use std::env;\n" +
            "+fn main() {\n" +
            '+    let who = env::args().nth(1).unwrap_or("world".into());\n' +
            '+    println!("hello {who}");\n' +
            "+}",
        },
      ],
    };
    this.snapshot.timeline.push({ item: "tool_call", id: edit });
    this.emit();
    await sleep(250);

    this.snapshot.pending_permission = {
      id: "perm-1",
      session: "mock-session",
      tool_call: tc,
      title: "Allow running `cargo build`?",
      options: [
        { id: "allow", label: "Allow", kind: "allow_once" },
        { id: "always", label: "Always allow", kind: "allow_always" },
        { id: "reject", label: "Reject", kind: "reject_once" },
      ],
    };
    this.emit();
    await sleep(50);

    const answer =
      "I read `src/main.rs`. It defines a `main` that prints a greeting. " +
      "Next I'd wire up argument parsing — want me to proceed?";
    for (const word of answer.split(" ")) {
      this.appendAgentText(run, word + " ");
      this.emit();
      await sleep(28);
    }

    this.snapshot.plan = {
      phases: [
        { title: "Inspect the workspace", status: "completed" },
        { title: "Apply the change", status: "pending" },
      ],
    };
    this.snapshot.runs[run] = {
      id: run,
      status: "done",
      outcome: { status: "done", stop_reason: "end_turn" },
    };
    this.emit();
  }

  private appendAgentText(run: string, text: string) {
    const last = this.snapshot.timeline[this.snapshot.timeline.length - 1];
    if (last && last.item === "message" && last.role === "agent" && last.run === run) {
      const lastBlock = last.blocks[last.blocks.length - 1];
      if (lastBlock && lastBlock.type === "text") {
        lastBlock.text += text;
        return;
      }
    }
    this.snapshot.timeline.push({
      item: "message",
      run,
      role: "agent",
      blocks: [{ type: "text", text }],
    });
  }
}
