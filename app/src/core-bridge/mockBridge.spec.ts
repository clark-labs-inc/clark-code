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
    expect(done.timeline.filter((t) => t.item === "plan").length).toBe(1);
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
});
