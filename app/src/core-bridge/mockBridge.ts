// Test-double bridge used in a plain browser (vite dev / Vitest). It emits
// pre-baked Snapshots to simulate a streaming agent run, so the UI is fully
// demonstrable without the native host. It deliberately does NOT re-implement
// the reducer — it just produces snapshots a real run would yield.

import type {
  CoreBridge,
  ConnectConfig,
  ManagedWorktree,
  ManagedWorktreeBranchReceipt,
  ManagedWorktreeCleanupReceipt,
  ManagedWorktreeRequest,
  PromptReceipt,
  ProjectBranch,
  ProjectContext,
  ProjectWorktreeTransitionPlan,
  SkillCatalogEntry,
  SkillCatalogSnapshot,
} from "./bridge";
import {
  emptySnapshot,
  type ClientResponse,
  type ContentBlock,
  type ProviderInfo,
  type SecurityScanRecord,
  type Session,
  type Snapshot,
} from "./types";
import { loadStoredResilienceCase, playResilienceSimulation } from "./resilienceBenchmark";
import { PRODUCT_SPECIALIST_CATALOG } from "../lib/specialists";
import {
  specialistConversationPresentation,
  specialistPresentationPayload,
} from "../lib/specialistPresentation";
import {
  FakeGitRepository,
  fakeGitScenario,
  type FakeManagedScenario,
} from "../lib/fakeGitRepository";

// Mirrors the shipped app: Agent Desktop is the only environment choice, while
// the product can route Scientist/RSI conversations through an internal
// native provider.
const PROVIDERS: ProviderInfo[] = [
  {
    id: "local",
    label: "Agent Desktop",
    capabilities: {
      streaming: true,
      permissions: true,
      fs: true,
      terminal: true,
      load_session: false,
      modes: [],
      collaboration_modes: ["default", "plan"],
    },
  },
  {
    id: "specialist",
    label: "the agent Specialist Runtime",
    internal: true,
    capabilities: {
      streaming: true,
      permissions: false,
      fs: false,
      terminal: false,
      load_session: true,
      modes: [],
      collaboration_modes: ["default"],
    },
  },
];

const SPECIALIST_SKILLS: SkillCatalogEntry[] = [
  ["scout:scout", "Map systems from bounded, evidence-backed investigation."],
  ["security:security-scan", "Assess repository security posture and validate findings."],
  ["security:security-diff", "Review a change set for security regressions."],
  ["security:security-deep", "Run a deep, multi-pass repository security scan."],
].map(([invocationName, description]) => ({
  id: `preview-${invocationName.replaceAll(":", "-")}`,
  revision: `preview-${invocationName.replaceAll(":", "-")}-v1`,
  name: invocationName.split(":").at(-1) ?? invocationName,
  invocationName,
  description,
  scope: "bundled",
  origin: "bundled",
  source: `skill://bundled/${invocationName.replace(":", "/")}`,
  requiredTools: [],
  missingTools: [],
  allowImplicitInvocation: false,
  enabled: true,
  disabledReason: null,
  hasNameCollision: false,
}));

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

function specialistPresentationForPrompt(userText: string) {
  const lower = userText.toLowerCase();
  const kind = lower.includes("archive-handling") || lower.includes("exploitable path") || lower.includes("deep scan")
    ? "security"
    : lower.includes("identity service") || lower.includes("blast radius")
      ? "scout"
      : lower.includes("latency") || lower.includes("replication") || lower.includes("falsification")
        ? "scientist"
        : lower.includes("counterexample") || lower.includes("evaluation world") || lower.includes("identity-loss")
          ? "rsi"
          : null;
  const presentation = kind ? specialistConversationPresentation(kind) : null;
  return presentation ? specialistPresentationPayload(presentation) : null;
}

export const SECURITY_SIMULATION_STORAGE_KEY =
  "agent-desktop:security-simulation";
/** Preview-only lifecycle state for the isolated-worktree manager. */
export const MANAGED_WORKTREE_SIMULATION_STORAGE_KEY =
  "agent-desktop:managed-worktree-simulation";

function managedWorktreeSimulation(): "ready" | "dirty" | "committed" {
  const value = localStorage.getItem(MANAGED_WORKTREE_SIMULATION_STORAGE_KEY);
  return value === "dirty" || value === "committed" ? value : "ready";
}

const simulatedFindings: NonNullable<SecurityScanRecord["seal"]>["findings"] = [
  ["SEC-ADMIN", "critical", "src/api/admin.ts", "Request body controls administrative access."],
  ["SEC-SQL", "critical", "src/db/search.ts", "Tenant input reaches an executable SQL string."],
  ["SEC-CMD", "critical", "src/jobs/export.ts", "Request fields reach a shell command."],
  ["SEC-SSRF", "high", "src/network/fetch.ts", "Tenant chooses a server-side network destination."],
].map(([candidateId, severity, sourcePath, impact], index) => ({
  findingId: `SEC-${String(index + 1).padStart(3, "0")}`,
  candidateId,
  severity: severity as "critical" | "high",
  sourcePath,
  impact,
}));

export function securitySimulationRecords(): SecurityScanRecord[] {
  const now = Date.UTC(2026, 6, 29, 12);
  return [
    {
      path: ".agent/security-scans/adversarial-standard/scan.json",
      modifiedAtMs: now,
      pocReceipts: [],
      bundle: {
        scanId: "adversarial-standard",
        mode: "standard",
        model: "local-model",
        scope: ".",
        inventoryId: "fixture-inventory",
        phase: "reporting",
        coverage: Array.from({ length: 21 }, (_, index) => ({
          path: `fixture/file-${index + 1}`,
          status: index === 20 ? "excluded" as const : "reviewed" as const,
          reason: index === 20 ? "Generated vendor code" : null,
        })),
        supportingCoverage: [],
        candidates: [],
      },
      seal: {
        scanId: "adversarial-standard",
        bundleDigest: "fixture-standard-digest",
        reviewedFiles: 20,
        excludedFiles: 1,
        supportingFiles: 0,
        findings: simulatedFindings,
      },
    },
    {
      path: ".agent/security-scans/adversarial-diff/scan.json",
      modifiedAtMs: now - 1_000,
      pocReceipts: [],
      bundle: {
        scanId: "adversarial-diff",
        mode: "diff",
        model: "local-model",
        scope: ".",
        inventoryId: "fixture-diff-inventory",
        phase: "reporting",
        coverage: [
          { path: "src/network/fetch.ts", status: "reviewed" },
          { path: "src/api/new-upload.ts", status: "reviewed" },
        ],
        supportingCoverage: [
          { path: "SECURITY.md", status: "reviewed" },
        ],
        candidates: [],
      },
      seal: {
        scanId: "adversarial-diff",
        bundleDigest: "fixture-diff-digest",
        reviewedFiles: 2,
        excludedFiles: 0,
        supportingFiles: 1,
        findings: simulatedFindings.slice(2),
      },
    },
    {
      path: ".agent/security-scans/adversarial-deep/scan.json",
      modifiedAtMs: now - 2_000,
      pocReceipts: [],
      bundle: {
        scanId: "adversarial-deep",
        mode: "deep",
        model: "local-model",
        scope: ".",
        inventoryId: "fixture-deep-inventory",
        phase: "reporting",
        coverage: Array.from({ length: 21 }, (_, index) => ({
          path: `fixture/file-${index + 1}`,
          status: "reviewed" as const,
        })),
        supportingCoverage: [],
        candidates: [],
      },
      seal: {
        scanId: "adversarial-deep",
        bundleDigest: "fixture-deep-digest",
        reviewedFiles: 21,
        excludedFiles: 0,
        supportingFiles: 0,
        deepPasses: 4,
        findings: simulatedFindings,
      },
    },
  ];
}

export class MockBridge implements CoreBridge {
  private snapshot: Snapshot = emptySnapshot();
  private handlers = new Set<(s: Snapshot) => void>();
  private git = new FakeGitRepository("/tmp/example-desktop", fakeGitScenario());
  private sessionSequence = 0;

  async listProviders(): Promise<ProviderInfo[]> {
    return PROVIDERS;
  }

  async listSpecialistCatalog() {
    return PRODUCT_SPECIALIST_CATALOG;
  }

  async openSession(
    providerId: string,
    _config: ConnectConfig,
    request: import("./bridge").SessionOpenRequest,
  ): Promise<Session> {
    const provider = PROVIDERS.find((p) => p.id === providerId) ?? PROVIDERS[0];
    if (request.kind === "load") {
      this.snapshot = { ...emptySnapshot(), session: request.id };
      return {
        id: request.id,
        provider: provider.id,
        capabilities: provider.capabilities,
        mode: provider.capabilities.modes[0],
        collaboration_mode: "default",
      };
    }
    const { options, bindId } = request;
    // Browser fixtures need a real list of distinct conversations. Keep the
    // original first id for existing demos, then make every new mock chat a
    // durable-looking sibling instead of silently replacing the first row.
    const id = bindId ?? (this.sessionSequence === 0 ? "mock-session" : `mock-session-${this.sessionSequence + 1}`);
    if (!bindId) this.sessionSequence += 1;
    this.snapshot = { ...emptySnapshot(), session: id };
    this.emit();
    return {
      id,
      provider: provider.id,
      capabilities: provider.capabilities,
      mode: options.mode,
      collaboration_mode: options.collaboration_mode ?? "default",
    };
  }

  async prompt(
    _sessionId: string,
    blocks: ContentBlock[],
    _attachments: import("../lib/attachments").Upload[] = [],
  ): Promise<PromptReceipt> {
    const userText = blocks
      .map((b) => b.type === "text" ? b.text : b.type === "skill_reference" ? "" : "[attachment]")
      .filter(Boolean)
      .join(" ");
    const runId = `run-${Date.now()}`;
    void this.playRun(userText, runId);
    return { runId };
  }

  async steer(_sessionId: string, blocks: ContentBlock[]): Promise<void> {
    const run = this.lastRunId();
    if (!run || this.snapshot.runs[run]?.status !== "running") {
      throw new Error("no active run to steer");
    }
    this.snapshot.timeline.push({ item: "message", run, role: "user", blocks });
    this.emit();
    if (this.snapshot.goal?.run === run && this.snapshot.goal.status === "active") {
      await this.playGoalSteer(run, blocks);
    }
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
    } else if (response.kind === "plan_decision" && this.snapshot.proposed_plan?.id === response.plan_id) {
      if (response.decision.action === "implement") {
        this.snapshot.proposed_plan = { ...this.snapshot.proposed_plan, status: "approved" };
      }
      this.emit();
    }
  }

  async setMode(_sessionId: string, _mode: string): Promise<void> {}

  async setCollaborationMode(_sessionId: string, _mode: "default" | "plan"): Promise<void> {}

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

  async listSkills(cwd: string): Promise<SkillCatalogSnapshot> {
    return {
      revision: "preview-specialists-v1",
      environmentId: "preview:local",
      projectRoot: cwd,
      skills: structuredClone(SPECIALIST_SKILLS),
      diagnostics: [],
    };
  }

  async reloadSkills(cwd: string): Promise<SkillCatalogSnapshot> {
    return this.listSkills(cwd);
  }

  async listSecurityScans(): Promise<SecurityScanRecord[]> {
    const mode =
      typeof localStorage === "undefined"
        ? "empty"
        : localStorage.getItem(SECURITY_SIMULATION_STORAGE_KEY) ?? "empty";
    if (mode === "error") {
      throw new Error("Simulated unreadable Security artifact");
    }
    return mode === "populated" ? securitySimulationRecords() : [];
  }

  async projectContext(cwd: string): Promise<ProjectContext | null> {
    if (!cwd.trim()) return null;
    const root = cwd.trim();
    // Browser previews can start with any persisted project folder, while the
    // original fixture repository used a fixed /tmp path. Re-root the test
    // double on first access so the same checkout context, branch ownership,
    // and parallel-agent signals are visible in the real preview too. Keep an
    // already-created managed path in the current repository intact.
    if (!this.git.context(root)) {
      this.git = new FakeGitRepository(root, fakeGitScenario());
    } else if (this.git.root === root) {
      this.git.setScenario(fakeGitScenario());
    }
    const context = this.git.context(root);
    if (!context) return null;
    const sessionActive = Boolean(this.snapshot.session);
    return {
      ...context,
      activity: {
        ...context.activity,
        changedFiles: sessionActive ? Math.max(2, context.activity.changedFiles) : context.activity.changedFiles,
        untrackedFiles: sessionActive ? Math.max(1, context.activity.untrackedFiles) : context.activity.untrackedFiles,
        externalAgents: sessionActive
          ? [
              {
                id: "external-preview",
                title: "Polish the shared checkout experience",
                agentNickname: "External agent",
                updatedAtMs: Date.now() - 18_000,
              },
            ]
          : [],
        detectedAtMs: Date.now(),
      },
    };
  }

  async openPath(): Promise<void> {
    /* no-op in the browser preview */
  }

  async listProjectBranches(projectPath: string): Promise<ProjectBranch[]> {
    return this.git.listBranches(projectPath);
  }

  async switchProjectBranch(projectPath: string, branch: string): Promise<void> {
    this.git.switchBranch(projectPath, branch);
  }

  async planProjectWorktree(
    projectPath: string,
    targetBranch?: string | null,
  ): Promise<ProjectWorktreeTransitionPlan> {
    return this.git.plan(projectPath, targetBranch);
  }

  async createManagedWorktree(
    projectPath: string,
    request: ManagedWorktreeRequest,
  ): Promise<ManagedWorktree> {
    return this.git.createManaged(projectPath, request, managedWorktreeSimulation() as FakeManagedScenario);
  }

  async listManagedWorktrees(projectPath: string): Promise<ManagedWorktree[]> {
    return this.git.listManaged(projectPath);
  }

  async cleanupManagedWorktree(
    projectPath: string,
    id: string,
  ): Promise<ManagedWorktreeCleanupReceipt> {
    return this.git.cleanupManaged(projectPath, id);
  }

  async saveManagedWorktreeBranch(
    projectPath: string,
    id: string,
  ): Promise<ManagedWorktreeBranchReceipt> {
    return this.git.saveManaged(projectPath, id);
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
  private async playRun(userText: string, run: string) {
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

    const resilienceCase = loadStoredResilienceCase();
    if (resilienceCase) {
      await playResilienceSimulation(resilienceCase, {
        snapshot: this.snapshot,
        run,
        emit: () => this.emit(),
        isCancelled: () => this.snapshot.runs[run]?.status === "cancelled",
        sleep,
      });
      return;
    }

    if (/^\s*\/goal(?:\s|$)/i.test(userText) || userText.toLowerCase().includes("goal simulation")) {
      await this.playGoalSimulation(run, userText);
      return;
    }

    const specialistPresentation = specialistPresentationForPrompt(userText);
    if (specialistPresentation) {
      await this.playSpecialistRun(run, specialistPresentation);
      return;
    }

    // Demo hook: "out of credits" reproduces the insufficient-credits failure so
    // the upgrade banner can be seen in the browser preview.
    if (userText.toLowerCase().includes("out of credits")) {
      this.snapshot.runs[run] = {
        id: run,
        status: "failed",
        outcome: {
          status: "failed",
          error: "insufficient_credits: out of the agent credits",
          failure_kind: "insufficient_credits",
        },
        checkpoint: "mock-checkpoint-sha",
      };
      this.emit();
      return;
    }

    this.snapshot.execution_checklist = {
      steps: [
        { title: "Inspect the workspace", status: "in_progress" },
        { title: "Apply the change", status: "pending" },
      ],
      revision: 1,
    };
    const planItem = this.snapshot.timeline.find((t) => t.item === "execution_checklist" && t.run === run);
    if (planItem?.item === "execution_checklist") {
      planItem.checklist = structuredClone(this.snapshot.execution_checklist);
    } else {
      this.snapshot.timeline.push({
        item: "execution_checklist",
        run,
        checklist: structuredClone(this.snapshot.execution_checklist),
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
            objective: "Trace the agent server route, auth, access, and artifact seams.",
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

    // A the agent research call — keep the cloud phase live in browser demos long
    // enough to inspect its compact progress surface before cited findings land.
    const research = `tc-research-${Date.now()}`;
    const xtermResearch = userText.toLowerCase().includes("xterm");
    const researchQuery = xtermResearch
      ? "xterm.js selection behavior and VS Code integration"
      : "latest clap argument-parsing API";
    this.snapshot.tool_calls[research] = {
      id: research,
      title: `brokered_research: ${researchQuery}`,
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
        "This document proposes improvements to how artifacts are surfaced, viewed, and connected to their generating context in Agent Desktop.",
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

    this.snapshot.execution_checklist = {
      steps: [
        { title: "Inspect the workspace", status: "completed" },
        { title: "Apply the change", status: "pending" },
      ],
      revision: 2,
    };
    const finalPlanItem = this.snapshot.timeline.find((t) => t.item === "execution_checklist" && t.run === run);
    if (finalPlanItem?.item === "execution_checklist") {
      finalPlanItem.checklist = structuredClone(this.snapshot.execution_checklist);
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

  /** Deterministic specialist fixture used by the browser GUI preview. It
   * follows the same timeline shape as a native provider: public narration,
   * a typed presentation item, then a concise terminal answer. */
  private async playSpecialistRun(
    run: string,
    presentation: ReturnType<typeof specialistPresentationPayload>,
  ) {
    this.snapshot.timeline.push({
      item: "message",
      run,
      role: "agent",
      phase: "commentary",
      blocks: [{
        type: "text",
        text: "I’ve assembled the evidence and decision surface so you can inspect the result, not just the narration.",
      }],
    });
    this.emit();
    await sleep(350);
    this.snapshot.timeline.push({
      item: "specialist_presentation",
      run,
      presentation,
    });
    this.emit();
    await sleep(500);
    this.snapshot.timeline.push({
      item: "message",
      run,
      role: "agent",
      phase: "final_answer",
      blocks: [{
        type: "text",
        text: "The presentation is ready. Use the view tabs to move from the map to supporting evidence and the run lifecycle.",
      }],
    });
    this.snapshot.runs[run] = {
      id: run,
      status: "done",
      outcome: { status: "done", stop_reason: "specialist_presentation" },
      checkpoint: "mock-checkpoint-sha",
    };
    this.emit();
  }

  /** Deterministic goal fixture for browser QA and product demos. The typed
   *  state and run-linked tool rows mirror the native provider boundary. */
  private async playGoalSimulation(run: string, userText: string) {
    const lower = userText.toLowerCase();
    const requestedStatus = lower.includes("complete")
      ? "complete" as const
      : lower.includes("budget")
        ? "budget_limited" as const
        : lower.includes("active")
          ? "active" as const
          : "blocked" as const;
    const goalId = "mock-goal";
    const commandObjective = userText.match(/^\s*\/goal\s+([\s\S]+)/i)?.[1]?.trim();
    const objective = commandObjective || "Fully implement and test the typed goal experience";
    this.snapshot.goal = {
      id: goalId,
      objective,
      status: "active",
      run,
      token_budget: 100_000,
      tokens_used: 18_420,
      time_used_seconds: 0,
      continuations: 0,
      updated_at_ms: Date.now(),
    };
    this.snapshot.timeline.push({
      item: "message",
      run,
      role: "agent",
      phase: "commentary",
      blocks: [{ type: "text", text: "I’m tracing the goal contract, persistence, and UI receipt before I wire the simulation." }],
    });
    this.emit();
    await sleep(120);

    const addedLines = Array.from({ length: 1_377 }, (_, index) => `+added line ${index + 1}`);
    const deletedLines = Array.from({ length: 427 }, (_, index) => `-deleted line ${index + 1}`);
    for (let index = 0; index < 24; index++) {
      const id = `goal-edit-${index + 1}`;
      const path = `app/src/goal/fixture-${index + 1}.ts`;
      this.snapshot.tool_calls[id] = {
        id,
        title: `Edit ${path}`,
        kind: "edit",
        status: "completed",
        locations: [{ path, line: 1 }],
        content: [{
          type: "text",
          text: [
            `diff ${path}`,
            ...(index === 0 ? [...deletedLines, ...addedLines] : ["-old", "+new"]),
          ].join("\n"),
        }],
      };
      this.snapshot.timeline.push({ item: "tool_call", id, run });
    }
    this.snapshot.timeline.push({
      item: "message",
      run,
      role: "system",
      blocks: [{ type: "text", text: "Goal turn 2: continuing toward the objective (18,420 tokens used)." }],
    });

    const blocker = "The paid provider evaluation needs an explicit model and spend cap.";
    this.snapshot.goal = {
      ...this.snapshot.goal,
      status: requestedStatus,
      tokens_used: 24_870,
      time_used_seconds: 43,
      continuations: 2,
      updated_at_ms: Date.now(),
      blocker_reason: requestedStatus === "blocked" ? blocker : undefined,
    };
    this.snapshot.timeline.push({
      item: "message",
      run,
      role: "agent",
      phase: requestedStatus === "active" ? "commentary" : "final_answer",
      blocks: [{
        type: "text",
        text: requestedStatus === "active"
          ? "The core goal flow is in place. I’m keeping the goal active so you can steer what I verify next."
          : requestedStatus === "blocked"
            ? "The typed goal flow, compact work receipt, persistence, and deterministic simulation are implemented and verified. The paid evaluation is configured but not run because live model calls require an explicit model and spend cap."
            : "The typed goal flow, compact work receipt, persistence, and deterministic simulation are implemented and verified.",
      }],
    });
    if (requestedStatus !== "active") {
      this.snapshot.runs[run] = {
        id: run,
        status: "done",
        outcome: { status: "done", stop_reason: "end_turn" },
        checkpoint: "mock-checkpoint-sha",
      };
    }
    this.emit();
  }

  /** Continue an active simulated goal from an explicit queued-message steer.
   *  The user turn is already echoed by `steer`; this adds visibly different
   *  plan/work evidence so browser QA proves trajectory change, not just that
   *  the button removed a queued chip. */
  private async playGoalSteer(run: string, blocks: ContentBlock[]) {
    const instruction = blocks
      .filter((block): block is Extract<ContentBlock, { type: "text" }> => block.type === "text")
      .map((block) => block.text.trim())
      .filter(Boolean)
      .join(" ");
    const accessibility = /accessib|keyboard|screen reader/i.test(instruction);
    const direction = accessibility
      ? "Prioritize accessibility and keyboard-navigation verification"
      : instruction || "Apply the user's updated direction";
    const path = accessibility
      ? "app/src/goal/accessibility-verification.ts"
      : "app/src/goal/steered-trajectory.ts";

    this.snapshot.execution_checklist = {
      steps: [
        { title: direction, status: "in_progress" },
        { title: "Verify the revised goal trajectory", status: "pending" },
      ],
      revision: 1,
    };
    this.snapshot.timeline.push({
      item: "execution_checklist",
      run,
      checklist: structuredClone(this.snapshot.execution_checklist),
    });
    this.snapshot.timeline.push({
      item: "message",
      run,
      role: "agent",
      phase: "commentary",
      blocks: [{
        type: "text",
        text: `Steering received — ${direction.toLowerCase()} before I complete the goal.`,
      }],
    });
    this.emit();
    await sleep(180);

    const id = `goal-steer-${Date.now()}`;
    this.snapshot.tool_calls[id] = {
      id,
      title: `Apply steer: ${path}`,
      kind: "edit",
      status: "completed",
      locations: [{ path, line: 1 }],
      content: [{
        type: "text",
        text: [
          `diff ${path}`,
          "+export const steeredGoalDirection =",
          `+  ${JSON.stringify(direction)};`,
        ].join("\n"),
      }],
    };
    this.snapshot.timeline.push({ item: "tool_call", id, run });
    this.snapshot.timeline.push({
      item: "message",
      run,
      role: "system",
      blocks: [{
        type: "text",
        text: `Goal turn 3: the user steer changed the active trajectory to “${direction}”.`,
      }],
    });
    if (this.snapshot.goal?.run === run) {
      const now = Date.now();
      const liveElapsed = Math.max(
        0,
        Math.floor((now - this.snapshot.goal.updated_at_ms) / 1_000),
      );
      this.snapshot.goal = {
        ...this.snapshot.goal,
        tokens_used: this.snapshot.goal.tokens_used + 1_320,
        time_used_seconds: this.snapshot.goal.time_used_seconds + liveElapsed,
        continuations: this.snapshot.goal.continuations + 1,
        updated_at_ms: now,
      };
    }
    this.snapshot.execution_checklist = {
      steps: [
        { title: direction, status: "completed" },
        { title: "Verify the revised goal trajectory", status: "in_progress" },
      ],
      revision: 2,
    };
    const planItem = this.snapshot.timeline.find(
      (item) => item.item === "execution_checklist" && item.run === run,
    );
    if (planItem?.item === "execution_checklist") {
      planItem.checklist = structuredClone(this.snapshot.execution_checklist);
    }
    this.snapshot.timeline.push({
      item: "message",
      run,
      role: "agent",
      phase: "commentary",
      blocks: [{
        type: "text",
        text: accessibility
          ? "The steer took effect: accessibility verification is now on the active path, and keyboard navigation is the next completion gate."
          : `The steer took effect: “${direction}” is now on the active path.`,
      }],
    });
    this.emit();
  }
}
