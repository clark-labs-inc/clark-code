import { beforeEach, describe, expect, it } from "vitest";
import {
  approvalPolicyForSpecialist,
  loadApprovalPolicy,
  loadApprovalPolicies,
  loadCollaborationMode,
  nextApprovalPolicy,
  saveApprovalPolicies,
  wouldAutoApprove,
} from "./permissions";
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

beforeEach(() => {
  const values = new Map<string, string>();
  (globalThis as { localStorage: Storage }).localStorage = {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
    clear: () => values.clear(),
    key: (index) => [...values.keys()][index] ?? null,
    get length() { return values.size; },
  } as Storage;
});

describe("wouldAutoApprove", () => {
  it("ask mode never auto-approves", () => {
    for (const r of [undefined, "safe", "caution", "danger"]) {
      expect(wouldAutoApprove("ask", req(r))).toBe(false);
    }
  });

  it("auto mode asks at caution, destructive, network, sandbox, external, and billed boundaries", () => {
    expect(wouldAutoApprove("auto", req("safe"))).toBe(true);
    expect(wouldAutoApprove("auto", req("caution"))).toBe(false);
    expect(wouldAutoApprove("auto", req(undefined))).toBe(true); // file edits
    expect(wouldAutoApprove("auto", req("danger"))).toBe(false); // asks
    expect(wouldAutoApprove("auto", req("network"))).toBe(false); // asks before host network
    expect(wouldAutoApprove("auto", req("sandbox"))).toBe(false); // asks before host access
    expect(wouldAutoApprove("auto", req("external"))).toBe(false); // MCP — asks
    expect(wouldAutoApprove("auto", req("billed"))).toBe(false); // image generation — asks
  });

  it("full mode approves local, destructive, website, external-tool, and billed actions", () => {
    for (const r of [
      undefined,
      "safe",
      "caution",
      "danger",
      "network",
      "sandbox",
      "external",
      "billed",
    ]) {
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

  it("entering plan mode is never auto-approved, in any mode", () => {
    for (const mode of ["ask", "auto", "full"] as const) {
      expect(wouldAutoApprove(mode, req("plan_entry"))).toBe(false);
    }
  });

  it("a cloud confirmation gate is never auto-approved, in any mode", () => {
    // The backend paused before an irreversible action — the pause exists to
    // get a human answer, so even "full" must not grant it.
    for (const mode of ["ask", "auto", "full"] as const) {
      expect(wouldAutoApprove(mode, req("confirm"))).toBe(false);
    }
  });
});

describe("nextApprovalPolicy", () => {
  it("cycles approval without mixing in collaboration mode", () => {
    expect(nextApprovalPolicy("ask")).toBe("auto");
    expect(nextApprovalPolicy("auto")).toBe("full");
    expect(nextApprovalPolicy("full")).toBe("ask");
  });
});

describe("specialist approval policy", () => {
  it("forces uninterrupted specialists to full access without changing others", () => {
    expect(approvalPolicyForSpecialist("ask", "scout")).toBe("full");
    expect(approvalPolicyForSpecialist("auto", "security")).toBe("auto");
    expect(approvalPolicyForSpecialist("ask", null)).toBe("ask");
  });
});

describe("legacy preference migration", () => {
  it("maps old plan mode to auto approval plus Plan collaboration", () => {
    localStorage.setItem("agent-desktop:permission-mode", "plan");
    expect(loadApprovalPolicy()).toBe("auto");
    expect(loadCollaborationMode()).toBe("plan");
  });
});

describe("per-conversation approval policies", () => {
  it("round-trips overrides and falls back to the global default for unknown ids", () => {
    saveApprovalPolicies({ "chat-a": "full", "chat-b": "ask" });
    const policies = loadApprovalPolicies();
    expect(policies["chat-a"]).toBe("full");
    expect(policies["chat-b"]).toBe("ask");
    // A chat with no override resolves to the global default.
    expect(policies["chat-c"]).toBeUndefined();
  });

  it("drops anything that is not a known policy", () => {
    saveApprovalPolicies({
      "chat-a": "full",
      // A future/typo value must not survive a round trip.
      "chat-b": "yolo" as unknown as "ask",
      "chat-c": 42 as unknown as "auto",
    });
    const policies = loadApprovalPolicies();
    expect(policies).toEqual({ "chat-a": "full" });
  });

  it("returns {} when nothing is stored", () => {
    expect(loadApprovalPolicies()).toEqual({});
  });
});
