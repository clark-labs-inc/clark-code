import { describe, expect, it } from "vitest";
import { contextLocationLabel } from "./ComposerContextBar";

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
