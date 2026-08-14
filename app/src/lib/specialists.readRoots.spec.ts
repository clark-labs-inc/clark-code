import { describe, expect, it } from "vitest";
import { isSpecComposerSession, specialistReadRoots } from "./specialists";

describe("specialistReadRoots", () => {
  it("binds Spec only to the repository the user selected", () => {
    expect(specialistReadRoots(
      { kind: "spec", repositoryPath: "/repos/clark" },
      ["/repos/other"],
    )).toEqual(["/repos/clark"]);
  });

  it("does not grant a repository before Spec has an explicit focus", () => {
    expect(specialistReadRoots({ kind: "spec" }, ["/repos/recent"])).toEqual([]);
  });

  it("preserves Scout's account-scoped census roots", () => {
    expect(specialistReadRoots(
      { kind: "scout" },
      ["/repos/payments", "/repos/identity"],
    )).toEqual(["/repos/payments", "/repos/identity"]);
  });
});

describe("isSpecComposerSession", () => {
  it("does not leak a background Spec session into the active Scout composer", () => {
    expect(isSpecComposerSession("scout")).toBe(false);
  });

  it("does not leak a background Spec session into New session", () => {
    expect(isSpecComposerSession(null)).toBe(false);
  });

  it("keeps the active Spec lens focused", () => {
    expect(isSpecComposerSession("spec")).toBe(true);
  });
});
