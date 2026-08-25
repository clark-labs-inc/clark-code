import { describe, expect, it } from "vitest";
import {
  MockBridge,
  MANAGED_WORKTREE_SIMULATION_STORAGE_KEY,
  SECURITY_SIMULATION_STORAGE_KEY,
  securitySimulationRecords,
} from "./mockBridge";
import type { Snapshot } from "./types";

function waitFor(predicate: (s: Snapshot) => boolean, bridge: MockBridge, ms = 4000) {
  return new Promise<Snapshot>((resolve, reject) => {
    let settled = false;
    // `subscribe` emits the current snapshot synchronously, so `unsub` may be
    // invoked before the assignment completes — default to a no-op and clean up
    // after subscription returns if the predicate matched immediately.
    let unsub: () => void = () => {};
    const finish = (fn: () => void) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      unsub();
      fn();
    };
    const timer = setTimeout(() => finish(() => reject(new Error("timeout"))), ms);
    unsub = bridge.subscribe((snap) => {
      if (predicate(snap)) finish(() => resolve(snap));
    });
    if (settled) unsub();
  });
}

describe("MockBridge", () => {
  it("simulates populated, empty, and failed Security history journeys", async () => {
    const bridge = new MockBridge();
    localStorage.setItem(SECURITY_SIMULATION_STORAGE_KEY, "populated");
    expect(await bridge.listSecurityScans()).toEqual(securitySimulationRecords());

    localStorage.setItem(SECURITY_SIMULATION_STORAGE_KEY, "empty");
    expect(await bridge.listSecurityScans()).toEqual([]);

    localStorage.setItem(SECURITY_SIMULATION_STORAGE_KEY, "error");
    await expect(bridge.listSecurityScans()).rejects.toThrow(
      "Simulated unreadable Security artifact",
    );
    localStorage.removeItem(SECURITY_SIMULATION_STORAGE_KEY);
  });

  it("exposes the local coding provider", async () => {
    const b = new MockBridge();
    const providers = await b.listProviders();
    expect(providers.map((p) => p.id)).toEqual(["local", "specialist"]);
    expect(providers.find((p) => p.id === "specialist")?.internal).toBe(true);
    expect(providers[0].capabilities.load_session).toBe(false);
  });

  it("exposes every specialist workflow in the browser preview", async () => {
    const catalog = await new MockBridge().listSkills("/tmp/example-desktop");

    expect(catalog.skills.map((skill) => skill.invocationName)).toEqual([
      "scout:scout",
      "security:security-scan",
      "security:security-diff",
      "security:security-deep",
    ]);
    expect(catalog.skills.every((skill) => skill.enabled)).toBe(true);
  });

  it("keeps specialist engine references out of the visible mock message", async () => {
    const bridge = new MockBridge();
    const session = await bridge.openSession("local", {}, { kind: "new", options: {} });

    await bridge.prompt(session.id, [
      { type: "text", text: "Deep scan this repository." },
      {
        type: "skill_reference",
        id: "preview-security-security-deep",
        revision: "preview-security-security-deep-v1",
        name: "security:security-deep",
      },
    ]);
    const snapshot = await waitFor(
      (value) => value.timeline.some((item) => item.item === "message" && item.role === "user"),
      bridge,
    );
    const message = snapshot.timeline.find((item) => item.item === "message" && item.role === "user");

    expect(message).toMatchObject({
      item: "message",
      blocks: [{ type: "text", text: "Deep scan this repository." }],
    });
  });

  it("renders a typed specialist presentation in the conversation timeline", async () => {
    const bridge = new MockBridge();
    const session = await bridge.openSession("local", {}, { kind: "new", options: {} });

    await bridge.prompt(session.id, [
      { type: "text", text: "Review the current archive-handling change for exploitable paths." },
    ]);
    const snapshot = await waitFor(
      (value) => value.timeline.some((item) => item.item === "specialist_presentation"),
      bridge,
    );
    const presentation = snapshot.timeline.find((item) => item.item === "specialist_presentation");

    expect(presentation).toMatchObject({
      item: "specialist_presentation",
      presentation: {
        kind: "security",
        title: "Archive extraction can cross the workspace boundary",
        diagram_title: "Validated attack path",
      },
    });
  });

  it("exposes checkout context for the browser preview", async () => {
    const context = await new MockBridge().projectContext("/tmp/example-desktop");

    expect(context).toEqual(expect.objectContaining({
      branch: "main",
      detached: false,
      isWorktree: false,
      worktreeRoot: "/tmp/example-desktop",
      activity: expect.objectContaining({
        changedFiles: 0,
        untrackedFiles: 0,
        conflictedFiles: 0,
        externalAgents: [],
      }),
    }));
  });

  it("re-roots the browser fixture for the persisted project folder", async () => {
    const bridge = new MockBridge();
    const root = "/Users/example/projects/example-desktop";

    expect(await bridge.projectContext(root)).toEqual(expect.objectContaining({
      branch: "main",
      worktreeRoot: root,
      activity: expect.objectContaining({ externalAgents: [] }),
    }));
    expect(await bridge.listProjectBranches(root)).toContainEqual({
      name: "feature/checkout-context",
      checkoutPath: null,
    });
  });

  it("switches the browser preview between known local branches", async () => {
    const bridge = new MockBridge();

    expect(await bridge.listProjectBranches("/tmp/example-desktop")).toContainEqual({
      name: "feature/checkout-context",
      checkoutPath: null,
    });
    await bridge.switchProjectBranch("/tmp/example-desktop", "feature/checkout-context");

    expect((await bridge.projectContext("/tmp/example-desktop"))?.branch).toBe(
      "feature/checkout-context",
    );
    await expect(
      bridge.switchProjectBranch("/tmp/example-desktop", "missing"),
    ).rejects.toThrow("Local branch missing no longer exists.");
  });

  it("models a managed worktree lifecycle without nesting it in preview", async () => {
    const bridge = new MockBridge();
    const source = "/tmp/example-desktop";
    const created = await bridge.createManagedWorktree(source, {
      base: "default",
      label: "review",
    });

    expect(await bridge.projectContext(created.path)).toMatchObject({
      detached: false,
      isWorktree: true,
      worktreeRoot: created.path,
    });
    await expect(bridge.createManagedWorktree(created.path, { base: "current" })).rejects.toThrow(
      "already a the agent-managed isolated worktree",
    );
    expect((await bridge.planProjectWorktree(created.path)).sourceIsManaged).toBe(true);
    expect(await bridge.listManagedWorktrees(created.path)).toEqual([created]);

    await expect(bridge.cleanupManagedWorktree(created.path, created.id)).resolves.toEqual({
      id: created.id,
      path: created.path,
      removed: true,
    });
    expect(await bridge.listManagedWorktrees(source)).toEqual([]);
  });

  it("simulates the save-commits-before-archive worktree journey", async () => {
    localStorage.setItem(MANAGED_WORKTREE_SIMULATION_STORAGE_KEY, "committed");
    const bridge = new MockBridge();
    const source = "/tmp/example-desktop";
    const created = await bridge.createManagedWorktree(source, { base: "default", label: "review" });

    expect(created).toMatchObject({ state: "committed", preservedBranch: `agent/${created.id}` });
    await expect(bridge.cleanupManagedWorktree(source, created.id)).rejects.toThrow(
      "not protected by a branch",
    );

    const saved = await bridge.saveManagedWorktreeBranch(source, created.id);
    expect(saved.branch).toBe(`agent/${created.id}-saved`);
    expect(await bridge.listManagedWorktrees(source)).toMatchObject([
      { id: created.id, state: "saved", preservedBranch: saved.branch },
    ]);
    await expect(bridge.cleanupManagedWorktree(source, created.id)).resolves.toMatchObject({
      removed: true,
    });
    localStorage.removeItem(MANAGED_WORKTREE_SIMULATION_STORAGE_KEY);
  });

  it("produces a streaming run with user + agent messages, a tool call and a plan", async () => {
    const b = new MockBridge();
    await b.openSession("local", {}, { kind: "new", options: {} });
    await b.prompt("mock-session", [{ type: "text", text: "look at main.rs" }]);

    const done = await waitFor(
      (s) => Object.values(s.runs).some((r) => r.status === "done"),
      b,
    );

    const roles = done.timeline
      .filter((t) => t.item === "message")
      .map((t) => (t.item === "message" ? t.role : ""));
    expect(roles).toContain("user");
    expect(roles).toContain("agent");

    expect(done.timeline.some((t) => t.item === "tool_call")).toBe(true);
    expect(done.timeline.some((t) => t.item === "execution_checklist")).toBe(true);
    expect(Object.keys(done.tool_calls).length).toBeGreaterThan(0);

    // Plan markers are de-duplicated to a single timeline entry.
    const plans = done.timeline.filter((t) => t.item === "execution_checklist");
    expect(plans.length).toBe(1);
    expect(plans[0].item === "execution_checklist" ? plans[0].checklist?.steps.length : 0).toBe(2);
  });

  it("clears the permission gate when resolved", async () => {
    const b = new MockBridge();
    await b.openSession("local", {}, { kind: "new", options: {} });
    await b.prompt("mock-session", [{ type: "text", text: "build it" }]);

    const gated = await waitFor((s) => !!s.pending_permission, b);
    expect(gated.pending_permission?.options.length).toBeGreaterThan(0);

    await b.respond("mock-session", {
      kind: "permission",
      request: gated.pending_permission!.id,
      option: "allow",
    });
    const cleared = await waitFor((s) => !s.pending_permission, b);
    expect(cleared.pending_permission).toBeUndefined();
  });

  it("echoes an explicit steering message into the active mock run", async () => {
    const b = new MockBridge();
    await b.openSession("local", {}, { kind: "new", options: {} });
    await b.prompt("mock-session", [{ type: "text", text: "build it" }]);

    await b.steer("mock-session", [{ type: "text", text: "use pnpm instead" }]);

    const steered = await waitFor(
      (s) => s.timeline.some(
        (item) => item.item === "message" && item.role === "user" &&
          item.blocks.some((block) => block.type === "text" && block.text === "use pnpm instead"),
      ),
      b,
    );
    expect(
      steered.timeline.filter((item) => item.item === "message" && item.role === "user"),
    ).toHaveLength(2);
  });

  it("emits linked artifacts for the browser artifact demo", async () => {
    const b = new MockBridge();
    await b.openSession("local", {}, { kind: "new", options: {} });
    await b.prompt("mock-session", [{ type: "text", text: "review artifact presentation" }]);

    const snapshot = await waitFor((s) => s.artifacts.length === 3, b);
    expect(snapshot.artifacts.map((artifact) => artifact.kind)).toEqual(["file", "image", "pdf"]);
    expect(snapshot.artifacts.every((artifact) => artifact.uri && artifact.tool_call)).toBe(true);
    expect(snapshot.artifacts[2].uri).toMatch(/^data:application\/pdf;base64,JVBERi0/);
    expect(snapshot.timeline.filter((item) => item.item === "artifact")).toHaveLength(3);
  });

  it("exposes a live product cloud research state before cited findings", async () => {
    const b = new MockBridge();
    await b.openSession("local", {}, { kind: "new", options: {} });
    await b.prompt("mock-session", [{ type: "text", text: "preview cloud research" }]);

    const snapshot = await waitFor(
      (state) => Object.values(state.tool_calls).some(
        (call) => call.kind === "research" && call.status === "in_progress",
      ),
      b,
    );
    const research = Object.values(snapshot.tool_calls).find((call) => call.kind === "research");
    expect(research?.raw_input).toEqual({ query: "latest clap argument-parsing API" });
    expect(research?.content).toEqual([]);
  });

  it("exposes typed parallel-work progress for the orchestration demo", async () => {
    const b = new MockBridge();
    await b.openSession("local", {}, { kind: "new", options: {} });
    await b.prompt("mock-session", [{ type: "text", text: "build this in parallel" }]);

    const snapshot = await waitFor((state) => state.fan_out?.running === 1, b);
    expect(snapshot.fan_out?.agents.map((agent) => agent.status)).toEqual([
      "done",
      "running",
      "queued",
    ]);
  });

  it("settles parallel descendants when the parent run is cancelled", async () => {
    const bridge = new MockBridge();
    await bridge.openSession("local", {}, { kind: "new", options: {} });
    await bridge.prompt("mock-session", [{ type: "text", text: "build this in parallel" }]);

    const active = await waitFor((state) => state.fan_out?.running === 1, bridge);
    const runId = Object.keys(active.runs)[0];
    await bridge.cancel();
    const cancelled = await waitFor(
      (state) => state.runs[runId]?.status === "cancelled",
      bridge,
    );

    expect(cancelled.runs[runId]?.status).toBe("cancelled");
    expect(cancelled.fan_out?.running).toBe(0);
    expect(cancelled.fan_out?.agents.map((agent) => agent.status)).toEqual([
      "done",
      "failed",
      "failed",
    ]);
    expect(cancelled.fan_out?.agents[0]?.result).toContain("Confirmed");
  });

  it("emits a typed blocked-goal receipt for deterministic UI simulation", async () => {
    const bridge = new MockBridge();
    await bridge.openSession("local", {}, { kind: "new", options: {} });
    await bridge.prompt("mock-session", [{ type: "text", text: "goal simulation blocked" }]);

    const snapshot = await waitFor((state) => state.goal?.status === "blocked", bridge);
    expect(snapshot.goal).toMatchObject({
      objective: "Fully implement and test the typed goal experience",
      status: "blocked",
      time_used_seconds: 43,
      continuations: 2,
    });
    expect(snapshot.timeline.filter((item) => item.item === "tool_call")).toHaveLength(24);
    expect(snapshot.timeline.find((item) => item.item === "tool_call")).toMatchObject({ run: snapshot.goal?.run });
  });

  it("visibly changes an active goal trajectory after an explicit steer", async () => {
    const bridge = new MockBridge();
    await bridge.openSession("local", {}, { kind: "new", options: {} });
    await bridge.prompt("mock-session", [{ type: "text", text: "goal simulation active" }]);

    const active = await waitFor(
      (state) => state.goal?.status === "active" &&
        state.timeline.filter((item) => item.item === "tool_call").length === 24,
      bridge,
    );
    expect(active.goal?.continuations).toBe(2);

    const steer = "Prioritize accessibility and keyboard navigation before completion.";
    await bridge.steer("mock-session", [{ type: "text", text: steer }]);
    const changed = await waitFor(
      (state) => Object.values(state.tool_calls).some(
        (call) => call.locations.some(
          (location) => location.path === "app/src/goal/accessibility-verification.ts",
        ),
      ),
      bridge,
    );

    expect(changed.goal).toMatchObject({ status: "active", continuations: 3 });
    expect(changed.timeline).toContainEqual(expect.objectContaining({
      item: "message",
      role: "user",
      blocks: [{ type: "text", text: steer }],
    }));
    expect(changed.execution_checklist?.steps).toEqual([
      expect.objectContaining({ title: "Prioritize accessibility and keyboard-navigation verification", status: "completed" }),
      expect.objectContaining({ title: "Verify the revised goal trajectory", status: "in_progress" }),
    ]);
    expect(changed.timeline).toContainEqual(expect.objectContaining({
      item: "message",
      role: "agent",
      blocks: [expect.objectContaining({ text: expect.stringContaining("The steer took effect") })],
    }));
  });

  it("clears an active goal from the simulated snapshot", async () => {
    const bridge = new MockBridge();
    await bridge.openSession("local", {}, { kind: "new", options: {} });
    await bridge.prompt("mock-session", [{ type: "text", text: "goal simulation active" }]);
    await waitFor(
      (state) => state.goal?.status === "active" &&
        state.timeline.filter((item) => item.item === "tool_call").length === 24,
      bridge,
    );

    await bridge.clearGoal("mock-session");

    const cleared = await waitFor((state) => state.goal === undefined, bridge);
    expect(cleared.goal).toBeUndefined();
    // Transcript is kept — only the goal receipt retires.
    expect(cleared.timeline.filter((item) => item.item === "tool_call")).toHaveLength(24);
  });
});
