import { describe, expect, it } from "vitest";
import { composerSubmissionState, detectComposerTrigger } from "./composerInput";

describe("detectComposerTrigger", () => {
  it("finds file and leading slash-command triggers", () => {
    expect(detectComposerTrigger("open @mai", 9)).toEqual({
      type: "@",
      query: "mai",
      start: 5,
    });
    expect(detectComposerTrigger("/goal", 5)).toEqual({ type: "/", query: "goal", start: 0 });
  });

  it("ignores mid-word and completed triggers", () => {
    expect(detectComposerTrigger("me@example.com", 14)).toBeNull();
    expect(detectComposerTrigger("@src done", 9)).toBeNull();
  });
});

describe("composerSubmissionState", () => {
  const ready = {
    hasContent: true,
    hasSession: false,
    connecting: false,
    activeProvider: "local",
    projectMode: "local" as const,
    localCwd: "/tmp/project",
    startBlocked: null,
    canPickProjectFolder: true,
  };

  it("lets a typed first message open the native folder picker", () => {
    expect(
      composerSubmissionState({
        ...ready,
        localCwd: "",
        startBlocked: "Choose a project folder.",
      }),
    ).toEqual({ canSubmit: true, shouldPickProjectFolder: true });
  });

  it("keeps unresolved start requirements disabled", () => {
    expect(
      composerSubmissionState({
        ...ready,
        projectMode: "remote",
        startBlocked: "Add a remote host.",
      }).canSubmit,
    ).toBe(false);
    expect(
      composerSubmissionState({
        ...ready,
        localCwd: "",
        startBlocked: "Choose a project folder.",
        canPickProjectFolder: false,
      }).canSubmit,
    ).toBe(false);
  });

  it("does not submit without content, a provider, or while connecting", () => {
    expect(composerSubmissionState({ ...ready, hasContent: false }).canSubmit).toBe(false);
    expect(composerSubmissionState({ ...ready, activeProvider: null }).canSubmit).toBe(false);
    expect(composerSubmissionState({ ...ready, connecting: true }).canSubmit).toBe(false);
  });
});
