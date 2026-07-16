import { describe, expect, it } from "vitest";
import { conversationProjectRoot, liveProjectRoot } from "./sessionEnvironment";

const session = (checkout?: string) => ({
  id: "session",
  provider: "local",
  capabilities: {
    streaming: true,
    permissions: true,
    fs: true,
    terminal: true,
    load_session: false,
    modes: [],
  },
  environment: checkout
    ? { checkout_root: checkout, workspace_roots: [checkout], remote: false }
    : undefined,
});

describe("session project affinity", () => {
  it("reopens in the persisted project instead of the mutable default", () => {
    expect(conversationProjectRoot("/repo/worktrees/a", "/repo/worktrees/b")).toBe(
      "/repo/worktrees/a",
    );
  });

  it("falls back for legacy conversations without project metadata", () => {
    expect(conversationProjectRoot(undefined, " /repo/default ")).toBe("/repo/default");
  });

  it("prefers the provider-reported canonical checkout", () => {
    expect(liveProjectRoot(session("/canonical/a"), "/uncanonical/a")).toBe("/canonical/a");
  });

  it("keeps the captured project for providers without environment metadata", () => {
    expect(liveProjectRoot(session(), "/repo/worktrees/a")).toBe("/repo/worktrees/a");
  });
});
