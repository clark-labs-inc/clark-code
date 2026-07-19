import { describe, expect, it } from "vitest";
import { MockBridge } from "./mockBridge";
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
  it("exposes the local coding provider", async () => {
    const b = new MockBridge();
    const providers = await b.listProviders();
    expect(providers.map((p) => p.id)).toEqual(["local"]);
    expect(providers[0].capabilities.load_session).toBe(false);
  });

  it("exposes checkout context for the browser preview", async () => {
    const context = await new MockBridge().projectContext("/tmp/clark-desktop");

    expect(context).toEqual({
      branch: "main",
      detached: false,
      isWorktree: false,
      worktreeRoot: "/tmp/clark-desktop",
    });
  });

  it("produces a streaming run with user + agent messages, a tool call and a plan", async () => {
    const b = new MockBridge();
    await b.newSession("local", {});
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
    expect(done.timeline.some((t) => t.item === "plan")).toBe(true);
    expect(Object.keys(done.tool_calls).length).toBeGreaterThan(0);

    // Plan markers are de-duplicated to a single timeline entry.
    const plans = done.timeline.filter((t) => t.item === "plan");
    expect(plans.length).toBe(1);
    expect(plans[0].item === "plan" ? plans[0].plan?.phases.length : 0).toBe(2);
  });

  it("clears the permission gate when resolved", async () => {
    const b = new MockBridge();
    await b.newSession("local", {});
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
    await b.newSession("local", {});
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
    await b.newSession("local", {});
    await b.prompt("mock-session", [{ type: "text", text: "review artifact presentation" }]);

    const snapshot = await waitFor((s) => s.artifacts.length === 3, b);
    expect(snapshot.artifacts.map((artifact) => artifact.kind)).toEqual(["file", "image", "pdf"]);
    expect(snapshot.artifacts[0].tool_call).toBeTruthy();
    expect(snapshot.timeline.filter((item) => item.item === "artifact")).toHaveLength(3);
  });

  it("exposes a live Clark Cloud research state before cited findings", async () => {
    const b = new MockBridge();
    await b.newSession("local", {});
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
    await b.newSession("local", {});
    await b.prompt("mock-session", [{ type: "text", text: "build this in parallel" }]);

    const snapshot = await waitFor((state) => state.fan_out?.running === 1, b);
    expect(snapshot.fan_out?.agents.map((agent) => agent.status)).toEqual([
      "done",
      "running",
      "queued",
    ]);
  });
});
