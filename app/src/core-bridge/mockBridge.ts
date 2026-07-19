// Test-double bridge used in a plain browser (vite dev / Vitest). It emits
// pre-baked Snapshots to simulate a streaming agent run, so the UI is fully
// demonstrable without the native host. It deliberately does NOT re-implement
// the reducer — it just produces snapshots a real run would yield.

import type { CoreBridge, ConnectConfig, ProjectContext, SessionOptions } from "./bridge";
import {
  emptySnapshot,
  type ClientResponse,
  type ContentBlock,
  type ProviderInfo,
  type Session,
  type Snapshot,
} from "./types";

// Mirrors the shipped app: one provider, the local coding agent (which has no
// server-side session to resume — load_session is false).
const PROVIDERS: ProviderInfo[] = [
  {
    id: "local",
    label: "Clark Code",
    capabilities: {
      streaming: true,
      permissions: true,
      fs: true,
      terminal: true,
      load_session: false,
      modes: [],
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

  async newSession(
    providerId: string,
    _options: SessionOptions,
    bindId?: string,
  ): Promise<Session> {
    const provider = PROVIDERS.find((p) => p.id === providerId) ?? PROVIDERS[0];
    const id = bindId ?? "mock-session";
    this.snapshot = { ...emptySnapshot(), session: id };
    this.emit();
    return {
      id,
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

  async steer(_sessionId: string, blocks: ContentBlock[]): Promise<void> {
    const run = this.lastRunId();
    if (!run || this.snapshot.runs[run]?.status !== "running") {
      throw new Error("no active run to steer");
    }
    this.snapshot.timeline.push({ item: "message", run, role: "user", blocks });
    this.emit();
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

  // A representative project tree so the @-mention picker is demoable in the
  // browser preview without a native file walk.
  async listFiles(): Promise<string[]> {
    return [
      "README.md",
      "package.json",
      "src/main.rs",
      "src/lib.rs",
      "src/store/sessionStore.ts",
      "src/surfaces/Composer.tsx",
      "src/surfaces/Conversation.tsx",
      "src/lib/fuzzy.ts",
      "tests/integration.rs",
    ];
  }

  async projectContext(cwd: string): Promise<ProjectContext | null> {
    if (!cwd.trim()) return null;
    return {
      branch: "main",
      detached: false,
      isWorktree: false,
      worktreeRoot: cwd.trim(),
    };
  }

  async openPath(): Promise<void> {
    /* no-op in the browser preview */
  }

  // `/btw` in the browser preview: a scripted answer after a short delay so
  // the overlay is demoable without a native provider. Never throws — a mock
  // failure would be indistinguishable from a real one in the UI.
  async sideQuestion(_sessionId: string, question: string): Promise<string> {
    await new Promise((r) => setTimeout(r, 700));
    return (
      `That's a side question — in the desktop app this is answered by a forked, tool-less ` +
      `model call over the current conversation context, without interrupting the active run.\n\n` +
      `> ${question}\n\n(Mock response — the real provider answers from the live session.)`
    );
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
    const parallelDemo = userText.toLowerCase().includes("parallel");
    this.snapshot.runs[run] = { id: run, status: "running", checkpoint: "mock-checkpoint-sha" };
    this.snapshot.timeline.push({
      item: "message",
      run,
      role: "user",
      blocks: [{ type: "text", text: userText }],
    });
    this.emit();
    await sleep(250);

    // Demo hook: "out of credits" reproduces the insufficient-credits failure so
    // the upgrade banner can be seen in the browser preview.
    if (userText.toLowerCase().includes("out of credits")) {
      this.snapshot.runs[run] = {
        id: run,
        status: "failed",
        outcome: {
          status: "failed",
          error: "insufficient_credits: out of Clark credits",
          failure_kind: "insufficient_credits",
        },
        checkpoint: "mock-checkpoint-sha",
      };
      this.emit();
      return;
    }

    this.snapshot.plan = {
      phases: [
        { title: "Inspect the workspace", status: "in_progress" },
        { title: "Apply the change", status: "pending" },
      ],
    };
    const planItem = this.snapshot.timeline.find((t) => t.item === "plan" && t.run === run);
    if (planItem?.item === "plan") {
      planItem.plan = structuredClone(this.snapshot.plan);
    } else {
      this.snapshot.timeline.push({
        item: "plan",
        run,
        plan: structuredClone(this.snapshot.plan),
      });
    }
    this.emit();
    await sleep(300);

    if (parallelDemo) {
      const now = Date.now();
      this.snapshot.fan_out = {
        title: "Map the provider path and validate local tool wiring",
        total: 3,
        done: 1,
        running: 1,
        agents: [
          {
            id: "platform-endpoint-survey",
            label: "Platform endpoint survey",
            status: "done",
            objective: "Trace the Clark server route, auth, billing, and artifact seams.",
            activity: "Complete",
            result: "Confirmed the platform route and authentication boundary.",
            attempt: 1,
            started_at_ms: now - 82_000,
            updated_at_ms: now - 21_000,
          },
          {
            id: "desktop-tool-wiring",
            label: "Desktop tool wiring",
            status: "running",
            objective: "Add typed local image tools without exposing provider credentials.",
            activity: "Reviewing the provider-local tool registry",
            attempt: 1,
            started_at_ms: now - 54_000,
            updated_at_ms: now,
          },
          {
            id: "image-workflow-verification",
            label: "Image workflow verification",
            status: "queued",
            objective: "Verify viewing, editing, and generated-image artifacts end to end.",
            activity: "Waiting to start",
            updated_at_ms: now,
          },
        ],
      };
      this.emit();
      // Keep the live state visible long enough to exercise selection, elapsed
      // time, and the inspector before the scripted run settles.
      await sleep(20_000);
    }

    this.snapshot.timeline.push({
      item: "message",
      run,
      role: "agent",
      phase: "commentary",
      blocks: [
        {
          type: "text",
          text: parallelDemo
            ? "I’ve mapped the provider path. Three bounded subagents are checking independent seams."
            : "I found the entrypoint. I’m checking the implementation path before I make the edit.",
        },
      ],
    });

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

    // A Clark research call — keep the cloud phase live in browser demos long
    // enough to inspect its compact progress surface before cited findings land.
    const research = `tc-research-${Date.now()}`;
    const xtermResearch = userText.toLowerCase().includes("xterm");
    const researchQuery = xtermResearch
      ? "xterm.js selection behavior and VS Code integration"
      : "latest clap argument-parsing API";
    this.snapshot.tool_calls[research] = {
      id: research,
      title: `clark_research: ${researchQuery}`,
      kind: "research",
      status: "in_progress",
      locations: [],
      raw_input: { query: researchQuery },
      content: [],
    };
    this.snapshot.timeline.push({ item: "tool_call", id: research });
    this.emit();
    await sleep(userText.toLowerCase().includes("research") ? 10_000 : 250);

    this.snapshot.tool_calls[research] = {
      ...this.snapshot.tool_calls[research],
      status: "completed",
      content: [
        {
          type: "text",
          text: xtermResearch
            ? "**xterm.js clears terminal selection through `SelectionService.clearSelection()`** " +
              "when input or buffer changes invalidate the current selection model.\n\n" +
              "- `onUserInput` clears an active selection before terminal input proceeds.\n" +
              "- Vertical buffer changes can reset selection through the `rowsChanged` path.\n" +
              "- VS Code preserves higher-level selection state around its terminal integration.\n\n" +
              "See the upstream source at https://github.com/xtermjs/xterm.js and the " +
              "API documentation at https://xtermjs.org/docs/api/terminal/classes/terminal/."
            : "**clap 4.x** is the current standard for argument parsing in Rust. The " +
              "derive API is recommended:\n\n" +
              "- Add `clap = { version = \"4\", features = [\"derive\"] }`\n" +
              "- Define a `#[derive(Parser)]` struct and call `Args::parse()`\n\n" +
              "The builder API remains available for dynamic cases. See the docs at " +
              "https://docs.rs/clap/latest/clap/ and the derive tutorial at " +
              "https://docs.rs/clap/latest/clap/_derive/_tutorial/index.html.",
        },
      ],
    };
    this.emit();
    await sleep(250);

    if (userText.toLowerCase().includes("artifact")) {
      const markdown = [
        "# Artifact UX recommendations",
        "",
        "This document proposes improvements to how artifacts are surfaced, viewed, and connected to their generating context in Clark Code.",
        "",
        "## What changed",
        "",
        "- First-class artifact tabs for focused reading and comparison",
        "- Persistent source linkage to the generating conversation turn",
        "- A context rail for details, versions, comments, and provenance",
        "",
        "## Why this model",
        "",
        "Artifacts are first-class outputs. Keeping them visible alongside their source helps users maintain trust, trace decisions, and iterate without losing context.",
        "",
        "| Approach | Focus | Benefit |",
        "| --- | --- | --- |",
        "| Inline only | Quick glance | Low friction |",
        "| Inspector | Side panel | Easy discovery |",
        "| Library | Dedicated workspace | Focused reading and comparison |",
      ].join("\n");
      const artifacts = [
        {
          id: "artifact-recommendations",
          title: "Artifact UX recommendations.md",
          kind: "file" as const,
          mime_type: "text/markdown",
          uri: `data:text/markdown;charset=utf-8,${encodeURIComponent(markdown)}`,
          tool_call: research,
        },
        {
          id: "artifact-sidebar",
          title: "artifact-sidebar.png",
          kind: "image" as const,
          mime_type: "image/png",
          tool_call: research,
        },
        {
          id: "artifact-summary",
          title: "Research summary.pdf",
          kind: "pdf" as const,
          mime_type: "application/pdf",
          tool_call: research,
        },
      ];
      this.snapshot.artifacts.push(...artifacts);
      this.snapshot.timeline.push(...artifacts.map((artifact) => ({ item: "artifact" as const, id: artifact.id })));
      this.emit();
      await sleep(150);
    }

    this.snapshot.pending_permission = {
      id: "perm-1",
      session: "mock-session",
      tool_call: tc,
      title: "Apply this edit?",
      detail:
        "diff src/main.rs\n@@ -1,3 +1,4 @@\n fn main() {\n-    println!(\"hi\");\n+    println!(\"hello, world\");\n+    parse_args();\n }",
      options: [
        { id: "allow_once", label: "Allow once", kind: "allow_once" },
        { id: "allow_always", label: "Always allow edits", kind: "allow_always" },
        { id: "reject_once", label: "Reject", kind: "reject_once" },
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
    const finalMessage = this.snapshot.timeline[this.snapshot.timeline.length - 1];
    if (finalMessage?.item === "message" && finalMessage.role === "agent") {
      finalMessage.phase = "final_answer";
    }

    this.snapshot.plan = {
      phases: [
        { title: "Inspect the workspace", status: "completed" },
        { title: "Apply the change", status: "pending" },
      ],
    };
    const finalPlanItem = this.snapshot.timeline.find((t) => t.item === "plan" && t.run === run);
    if (finalPlanItem?.item === "plan") {
      finalPlanItem.plan = structuredClone(this.snapshot.plan);
    }
    if (parallelDemo && this.snapshot.fan_out) {
      this.snapshot.fan_out = {
        ...this.snapshot.fan_out,
        done: 3,
        running: 0,
        agents: this.snapshot.fan_out.agents.map((agent) => ({
          ...agent,
          status: "done",
          activity: "Complete",
          result: agent.result ?? "Completed the delegated task.",
          updated_at_ms: Date.now(),
        })),
      };
    }
    this.snapshot.runs[run] = {
      id: run,
      status: "done",
      outcome: { status: "done", stop_reason: "end_turn" },
      checkpoint: "mock-checkpoint-sha",
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
