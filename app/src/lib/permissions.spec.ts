import { describe, expect, it } from "vitest";
import { nextPermissionMode, wouldAutoApprove } from "./permissions";
import type { PermissionRequest } from "../core-bridge/types";

function req(risk?: string): PermissionRequest {
  return {
    id: "p",
    session: "s",
    title: "Run a shell command?",
    risk,
    options: [{ id: "allow_once", label: "Allow once", kind: "allow_once" }],
  };
}

describe("wouldAutoApprove", () => {
  it("ask mode never auto-approves", () => {
    for (const r of [undefined, "safe", "caution", "danger"]) {
      expect(wouldAutoApprove("ask", req(r))).toBe(false);
    }
  });

  it("auto mode approves all but destructive + external tools", () => {
    expect(wouldAutoApprove("auto", req("safe"))).toBe(true);
    expect(wouldAutoApprove("auto", req("caution"))).toBe(true);
    expect(wouldAutoApprove("auto", req(undefined))).toBe(true); // file edits
    expect(wouldAutoApprove("auto", req("danger"))).toBe(false); // asks
    expect(wouldAutoApprove("auto", req("external"))).toBe(false); // MCP — asks
  });

  it("full mode approves everything (engine still blocks catastrophic)", () => {
    for (const r of [undefined, "safe", "caution", "danger"]) {
      expect(wouldAutoApprove("full", req(r))).toBe(true);
    }
  });

  it("never approves a request with no allow option", () => {
    const noAllow: PermissionRequest = {
      id: "p",
      session: "s",
      title: "x",
      risk: "safe",
      options: [{ id: "reject_once", label: "Reject", kind: "reject_once" }],
    };
    expect(wouldAutoApprove("full", noAllow)).toBe(false);
  });

  it("a plan approval is never auto-approved, in any mode", () => {
    for (const mode of ["ask", "auto", "full", "plan"] as const) {
      expect(wouldAutoApprove(mode, req("plan"))).toBe(false);
    }
  });

  it("entering plan mode is never auto-approved, in any mode", () => {
    for (const mode of ["ask", "auto", "full", "plan"] as const) {
      expect(wouldAutoApprove(mode, req("plan_entry"))).toBe(false);
    }
  });

  it("a cloud confirmation gate is never auto-approved, in any mode", () => {
    // The backend paused before an irreversible action — the pause exists to
    // get a human answer, so even "full" must not grant it.
    for (const mode of ["ask", "auto", "full", "plan"] as const) {
      expect(wouldAutoApprove(mode, req("confirm"))).toBe(false);
    }
  });
});

describe("nextPermissionMode", () => {
  it("cycles ask -> auto -> full -> plan -> ask", () => {
    expect(nextPermissionMode("ask")).toBe("auto");
    expect(nextPermissionMode("auto")).toBe("full");
    expect(nextPermissionMode("full")).toBe("plan");
    expect(nextPermissionMode("plan")).toBe("ask");
  });
});
