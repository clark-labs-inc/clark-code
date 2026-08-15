import { describe, expect, it } from "vitest";
import {
  composerContextKind,
  contextLocationLabel,
  hasSessionContextAuthority,
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
  it("keeps Scout enterprise-scoped and gives Spec an execution target", () => {
    expect(composerContextKind("scout")).toBe("enterprise");
    expect(composerContextKind("spec")).toBe("spec");
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
