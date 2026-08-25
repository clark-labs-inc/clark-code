import { describe, expect, it } from "vitest";
import {
  composerContextKind,
  contextLocationLabel,
  hasSessionContextAuthority,
  shouldInspectProjectContext,
} from "./ComposerContextBar";

describe("composer execution location", () => {
  it("never labels a local session with the selected SSH host", () => {
    expect(contextLocationLabel({
      isRemoteContext: false,
      activeRemoteHost: null,
      selectedRemoteHost: "ubuntu@nucleus",
    })).toBe("Local");
  });

  it("uses the active remote host before the pending selection", () => {
    expect(contextLocationLabel({
      isRemoteContext: true,
      activeRemoteHost: "ubuntu@active",
      selectedRemoteHost: "ubuntu@pending",
    })).toBe("ubuntu@active");
  });
});

describe("composer context authority", () => {
  it("keeps Scout enterprise-scoped and ordinary sessions on the checkout context", () => {
    expect(composerContextKind("scout")).toBe("enterprise");
    expect(composerContextKind(null)).toBe("checkout");
  });

  it("keeps a remote specialist authority visible without a checkout root", () => {
    expect(hasSessionContextAuthority({
      activeProvider: "local",
      checkoutRoot: "",
      activeRemoteHost: "engineer@example.invalid",
    })).toBe(true);
  });

  it("does not invent context for a rootless local session", () => {
    expect(hasSessionContextAuthority({
      activeProvider: "local",
      checkoutRoot: "",
      activeRemoteHost: null,
    })).toBe(false);
  });
});

describe("project context inspection consent", () => {
  const local = {
    activeSpecialist: null,
    activeProvider: "local",
    cwd: "/Users/test/Documents/project",
    hasSession: false,
    projectMode: "local" as const,
    remoteReady: false,
    authorizedLocalRoot: null,
  };

  it("does not probe a remembered local project on the start screen", () => {
    expect(shouldInspectProjectContext(local)).toBe(false);
  });

  it("probes after an explicit folder interaction", () => {
    expect(shouldInspectProjectContext({
      ...local,
      authorizedLocalRoot: local.cwd,
    })).toBe(true);
  });

  it("probes the checkout owned by an active session", () => {
    expect(shouldInspectProjectContext({ ...local, hasSession: true })).toBe(true);
  });

  it("keeps remote inspection available after the remote connection is ready", () => {
    expect(shouldInspectProjectContext({
      ...local,
      projectMode: "remote",
      remoteReady: true,
    })).toBe(true);
  });
});
