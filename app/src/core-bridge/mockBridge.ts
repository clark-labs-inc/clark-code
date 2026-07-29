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
  SessionOptions,
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
      collaboration_modes: ["default", "plan"],
    },
  },
];

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

export const SECURITY_SIMULATION_STORAGE_KEY =
  "clark-desktop:security-simulation";
/** Preview-only lifecycle state for the isolated-worktree manager. */
export const MANAGED_WORKTREE_SIMULATION_STORAGE_KEY =
  "clark-desktop:managed-worktree-simulation";

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
      path: ".clark/security-scans/adversarial-standard/scan.json",
      modifiedAtMs: now,
      pocReceipts: [],
      bundle: {
        scanId: "adversarial-standard",
        mode: "standard",
        model: "clark-code",
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
      path: ".clark/security-scans/adversarial-diff/scan.json",
      modifiedAtMs: now - 1_000,
      pocReceipts: [],
      bundle: {
        scanId: "adversarial-diff",
        mode: "diff",
        model: "clark-code",
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
      path: ".clark/security-scans/adversarial-deep/scan.json",
      modifiedAtMs: now - 2_000,
      pocReceipts: [],
      bundle: {
        scanId: "adversarial-deep",
        mode: "deep",
        model: "clark-code",
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
  private branch = "main";
  private managedWorktrees: ManagedWorktree[] = [];
  private managedSequence = 0;
  private sessionSequence = 0;

  async listProviders(): Promise<ProviderInfo[]> {
    return PROVIDERS;
  }

  async connect(_providerId: string, _config: ConnectConfig): Promise<void> {}

  async newSession(
    providerId: string,
    options: SessionOptions,
    bindId?: string,
  ): Promise<Session> {
    const provider = PROVIDERS.find((p) => p.id === providerId) ?? PROVIDERS[0];
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

  async loadSession(providerId: string, id: string): Promise<Session> {
    // The mock has no server state; the store restores the persisted snapshot.
    const provider = PROVIDERS.find((p) => p.id === providerId) ?? PROVIDERS[0];
    this.snapshot = { ...emptySnapshot(), session: id };
    return {
      id,
      provider: provider.id,
      capabilities: provider.capabilities,
      mode: provider.capabilities.modes[0],
      collaboration_mode: "default",
    };
  }

  async prompt(
    _sessionId: string,
    blocks: ContentBlock[],
    _attachments: import("../lib/attachments").Upload[] = [],
  ): Promise<PromptReceipt> {
    const userText = blocks
      .map((b) => (b.type === "text" ? b.text : "[attachment]"))
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
    const managed = this.managedWorktrees.find((worktree) => worktree.path === root);
    const sessionActive = Boolean(this.snapshot.session);
    return {
      branch: managed?.preservedBranch || this.branch,
      detached: managed?.state === "committed",
      isWorktree: Boolean(managed),
      worktreeRoot: root,
      activity: {
        changedFiles: sessionActive ? 2 : 0,
        untrackedFiles: sessionActive ? 1 : 0,
        conflictedFiles: 0,
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
    return ["main", "feature/checkout-context", "fix/composer-layout"].map((name) => ({
      name,
      checkoutPath: name === this.branch ? projectPath : null,
    }));
  }

  async switchProjectBranch(projectPath: string, branch: string): Promise<void> {
    if (
      !(await this.listProjectBranches(projectPath)).some(
        (candidate) => candidate.name === branch,
      )
    ) {
      throw new Error(`Local branch ${branch} no longer exists.`);
    }
    this.branch = branch;
  }

  async planProjectWorktree(
    projectPath: string,
    targetBranch?: string | null,
  ): Promise<ProjectWorktreeTransitionPlan> {
    const root = projectPath.trim();
    const target = targetBranch?.trim() || null;
    const managed = this.managedWorktrees.find((worktree) => worktree.path === root);
    const repositoryRoot = managed?.sourceRoot ?? root;
    const owner = target && target === this.branch && !managed ? root : null;
    return {
      sourceRoot: root,
      sourceBranch: this.branch,
      sourceRevision: "0123456789abcdef0123456789abcdef01234567",
      sourceChanges: { changedFiles: 0, untrackedFiles: 0, conflictedFiles: 0 },
      sourceIsManaged: Boolean(managed),
      targetBranch: target,
      targetCheckoutPath: owner,
      action: target && target !== this.branch ? "switch_clean" : "create_isolated",
      preservation: "clean",
      requiresConfirmation: false,
      baseOptions: [
        {
          id: "current",
          label: "Current checkout (" + this.branch + ")",
          reference: this.branch,
          revision: "0123456789abcdef0123456789abcdef01234567",
          fallback: false,
        },
        {
          id: "default",
          label: "Fresh default branch (origin/main)",
          reference: "origin/main",
          revision: "0123456789abcdef0123456789abcdef01234567",
          fallback: false,
        },
      ],
      managedLocation: repositoryRoot + ".clark-worktrees",
    };
  }

  async createManagedWorktree(
    projectPath: string,
    request: ManagedWorktreeRequest,
  ): Promise<ManagedWorktree> {
    const sourceRoot = projectPath.trim();
    if (this.managedWorktrees.some((worktree) => worktree.path === sourceRoot)) {
      throw new Error(
        "This checkout is already a Clark-managed isolated worktree. Reuse it instead of nesting another checkout.",
      );
    }
    const id = "session-" + ++this.managedSequence;
    const managedBranch = `clark/${id}`;
    const simulation = managedWorktreeSimulation();
    const baseRevision = "0123456789abcdef0123456789abcdef01234567";
    const privateHead = "fedcba9876543210fedcba9876543210fedcba98";
    const created: ManagedWorktree = {
      id,
      label: request.label?.trim() || "session",
      path: sourceRoot + ".clark-worktrees/" + id,
      sourceRoot,
      base: request.base,
      baseReference: request.targetBranch?.trim() || (request.base === "default" ? "origin/main" : this.branch),
      baseRevision,
      headRevision: simulation === "committed" ? privateHead : baseRevision,
      preservedBranch: managedBranch,
      createdAtMs: Date.now(),
      state: simulation,
      changes: simulation === "dirty"
        ? { changedFiles: 1, untrackedFiles: 0, conflictedFiles: 0 }
        : { changedFiles: 0, untrackedFiles: 0, conflictedFiles: 0 },
    };
    this.managedWorktrees.unshift(created);
    return created;
  }

  async listManagedWorktrees(projectPath: string): Promise<ManagedWorktree[]> {
    const sourceRoot = this.managedRepositoryRoot(projectPath);
    return this.managedWorktrees.filter((worktree) => worktree.sourceRoot === sourceRoot);
  }

  async cleanupManagedWorktree(
    projectPath: string,
    id: string,
  ): Promise<ManagedWorktreeCleanupReceipt> {
    const sourceRoot = this.managedRepositoryRoot(projectPath);
    const worktree = this.managedWorktrees.find(
      (candidate) => candidate.sourceRoot === sourceRoot && candidate.id === id,
    );
    if (!worktree) throw new Error("That managed worktree is not registered for this repository.");
    if (worktree.state !== "ready" && worktree.state !== "saved") {
      throw new Error(
        worktree.state === "committed"
          ? "Managed worktree has commits that are not protected by a branch."
          : "Managed worktree has local changes.",
      );
    }
    this.managedWorktrees = this.managedWorktrees.filter((candidate) => candidate !== worktree);
    return { id, path: worktree.path, removed: true };
  }

  async saveManagedWorktreeBranch(
    projectPath: string,
    id: string,
  ): Promise<ManagedWorktreeBranchReceipt> {
    const sourceRoot = this.managedRepositoryRoot(projectPath);
    const worktree = this.managedWorktrees.find(
      (candidate) => candidate.sourceRoot === sourceRoot && candidate.id === id,
    );
    if (!worktree) throw new Error("That managed worktree is not registered for this repository.");
    if (worktree.state === "dirty") {
      throw new Error("Managed worktree still has local changes.");
    }
    if (worktree.state !== "committed" && worktree.state !== "saved") {
      throw new Error("This managed worktree has no new commits to save as a branch.");
    }
    const branch = worktree.state === "committed" && worktree.preservedBranch
      ? `${worktree.preservedBranch}-saved`
      : worktree.preservedBranch || `clark/${worktree.id}`;
    worktree.preservedBranch = branch;
    worktree.state = "saved";
    return {
      id: worktree.id,
      path: worktree.path,
      branch,
      headRevision: worktree.headRevision || worktree.baseRevision,
    };
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

  private managedRepositoryRoot(projectPath: string): string {
    const root = projectPath.trim();
    return this.managedWorktrees.find((worktree) => worktree.path === root)?.sourceRoot ?? root;
  }

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
