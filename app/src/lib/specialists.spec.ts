import { describe, expect, it } from "vitest";
import {
  projectedSpecialistAccess,
  specialistAccessAfterLoadFailure,
  specialistAccessBadge,
  specialistDeepLink,
  specialistSlashIntent,
  specialistAccessCopy,
  specialistConnectConfig,
  specialistWorkflowAvailable,
  withActiveSpecialistSkill,
  SPECIALIST_KINDS,
} from "./specialists";
import type { BillingSummary } from "./billing";

function billing(
  status?: string,
  owner: "user" | "organization" = "user",
  assigned: boolean | null = null,
): BillingSummary {
  return {
    stripe_enabled: true,
    enforcement_enabled: true,
    effective: {
      owner_kind: owner,
      display_name: owner === "user" ? "Personal" : "Clark Labs",
      subscription: status ? { status } : null,
      seat: owner === "organization"
        ? { purchased: 3, assigned: 2, assigned_to_current_user: assigned }
        : null,
    },
  };
}

describe("specialist subscription access", () => {
  it("keeps Free signed-in accounts out even when coding credits may exist", () => {
    expect(projectedSpecialistAccess(true, billing())).toBe("free");
  });

  it("allows active and trial personal subscriptions", () => {
    expect(projectedSpecialistAccess(true, billing("active"))).toBe("ready");
    expect(projectedSpecialistAccess(true, billing("trialing"))).toBe("ready");
  });

  it("requires the current user's paid workspace seat", () => {
    expect(projectedSpecialistAccess(true, billing("active", "organization", true))).toBe("ready");
    expect(projectedSpecialistAccess(true, billing("active", "organization", false))).toBe("free");
  });

  it("does not project ready from an active plan when current coverage is unusable", () => {
    const unavailable = billing("active");
    unavailable.effective!.coverage_status = "unavailable";
    expect(projectedSpecialistAccess(true, unavailable)).toBe("action_needed");

    const limited = billing("active");
    limited.effective!.coverage_status = "ready";
    limited.effective!.access_state = "usage_limited";
    expect(projectedSpecialistAccess(true, limited)).toBe("action_needed");

    const wrongProduct = billing("active");
    wrongProduct.effective!.coverage_status = "ready";
    wrongProduct.effective!.products = ["clark_web"];
    expect(projectedSpecialistAccess(true, wrongProduct)).toBe("free");
  });

  it("preserves value-oriented Free copy", () => {
    expect(specialistAccessCopy("free", "security")).toEqual({
      title: "Unlock Clark Security",
      detail: "Clark Security is available with Pro coverage. Verified findings, safe PoCs, and remediation. Your existing chats and Clark Code remain available.",
      action: "upgrade",
    });
    expect(specialistAccessBadge("ready")).toBe("Access ready");
    expect(specialistAccessBadge("free")).toBe("Not included");
    expect(specialistAccessBadge("loading")).toBe("Checking access");
    expect(specialistAccessBadge("action_needed")).toBe("Action needed");
    expect(specialistAccessBadge("offline")).toBe("Can't verify access");
  });

  it("does not turn a post-entitlement data failure into a coverage failure", () => {
    expect(specialistAccessAfterLoadFailure(false)).toBe("offline");
    expect(specialistAccessAfterLoadFailure(true)).toBe("ready");
  });
});

describe("specialist engine references", () => {
  it("adds the pinned Scout workflow without changing the visible prompt", () => {
    expect(withActiveSpecialistSkill([], [{
      id: "skill-1",
      revision: "sha256:one",
      invocationName: "scout:scout",
      enabled: true,
    }], "scout")).toEqual([{
      type: "skill_reference",
      id: "skill-1",
      revision: "sha256:one",
      name: "scout:scout",
    }]);
    expect(specialistWorkflowAvailable([], "security", "security:security-deep")).toBe(false);
    expect(specialistWorkflowAvailable([], "scientist", "scientist:discover")).toBe(true);
  });
});

describe("specialist registry", () => {
  it("discovers Scientist and RSI without surface-specific branching", () => {
    expect(SPECIALIST_KINDS).toEqual([
      "scout",
      "security",
      "scientist",
      "rsi",
    ]);
  });

  it("builds a path-free native runtime request for Scientist", () => {
    expect(specialistConnectConfig(
      {
        kind: "scientist",
        organizationId: "org-1",
        workflow: "scientist:replicate",
      },
      "/tmp/project",
    )).toEqual({
      cwd: "/tmp/project",
      extra: {
        specialist: "scientist",
        workflow: "scientist:replicate",
        organizationId: "org-1",
        modelRoute: "clark_deepseek_v4_latest",
        maxIterations: 3,
      },
    });
  });

  it("rejects a workflow owned by another specialist", () => {
    expect(() => specialistConnectConfig(
      { kind: "scientist", workflow: "rsi:stress-test" },
      "/tmp/project",
    )).toThrow("Unsupported Scientist workflow");
  });
});

describe("specialist deep links", () => {
  it("accepts only tabs owned by the selected specialist", () => {
    expect(specialistDeepLink("?specialist=security&tab=findings")).toEqual({
      kind: "security",
      tab: "findings",
    });
    expect(specialistDeepLink("?specialist=scout&tab=findings")).toEqual({ kind: "scout" });
    expect(specialistDeepLink("?specialist=scientist&tab=experiments")).toEqual({
      kind: "scientist",
      tab: "experiments",
    });
  });
});

describe("legacy slash handoff", () => {
  it("routes deep security work into the Security scans canvas", () => {
    expect(specialistSlashIntent("/security-deep audit auth")).toEqual({
      kind: "security",
      tab: "scans",
      prompt: "Run a deep security scan. audit auth",
      workflow: "security:security-deep",
    });
  });

  it("accepts persisted collision-safe skill tokens during migration", () => {
    expect(specialistSlashIntent("$scout:scout map AWS")).toEqual({
      kind: "scout",
      tab: "map",
      prompt: "map AWS",
      workflow: "scout:scout",
    });
  });

  it("routes legacy simulation intent into RSI without a second product", () => {
    expect(specialistSlashIntent("/simulate identity outage")).toEqual({
      kind: "rsi",
      tab: "frontier",
      prompt: "identity outage",
      workflow: "rsi:stress-test",
    });
  });
});
