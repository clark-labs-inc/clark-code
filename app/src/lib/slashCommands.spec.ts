import { describe, expect, it } from "vitest";
import {
  expandPromptSlashCommand,
  goalCommandObjective,
  isCompactCommand,
  sideQuestionCommandQuestion,
  slashCommands,
  subscriptionWorkflowForSubmission,
} from "./slashCommands";

describe("goal slash command", () => {
  it("is discoverable as a local built-in that keeps the prefix editable", () => {
    expect(slashCommands()).toContainEqual(expect.objectContaining({
      name: "goal",
      body: "/goal",
      localOnly: true,
    }));
  });

  it("matches only an exact command prefix", () => {
    expect(goalCommandObjective(" /goal finish the feature ")).toBe("finish the feature");
    expect(goalCommandObjective("/goal")).toBe("");
    expect(goalCommandObjective("/goals list")).toBeNull();
    expect(goalCommandObjective("please /goal later")).toBeNull();
  });
});

describe("compact slash command", () => {
  it("is a local session action rather than prompt text", () => {
    expect(slashCommands()).toContainEqual(expect.objectContaining({
      name: "compact",
      needsSession: true,
      localOnly: true,
      run: expect.any(Function),
    }));
  });

  it("matches only the exact command without arguments", () => {
    expect(isCompactCommand("/compact")).toBe(true);
    expect(isCompactCommand("  /compact  ")).toBe(true);
    expect(isCompactCommand("/compact now")).toBe(false);
    expect(isCompactCommand("/compaction")).toBe(false);
    expect(isCompactCommand("please /compact")).toBe(false);
  });
});

describe("sentry slash command", () => {
  it("is discoverable as a local prompt command using the collision-safe skill name", () => {
    expect(slashCommands()).toContainEqual(expect.objectContaining({
      name: "sentry",
      body: "$sentry:sentry",
      localOnly: true,
    }));
  });

  it("expands direct command input without matching lookalike commands", () => {
    expect(expandPromptSlashCommand("/sentry")).toBe("$sentry:sentry");
    expect(expandPromptSlashCommand("  /sentry APP-123")).toBe("  $sentry:sentry APP-123");
    expect(expandPromptSlashCommand("/sentryish APP-123")).toBe("/sentryish APP-123");
    expect(expandPromptSlashCommand("inspect /sentry APP-123")).toBe("inspect /sentry APP-123");
  });
});

describe("scout slash command", () => {
  it("is discoverable as a handoff into the first-class Scout space", () => {
    expect(slashCommands()).toContainEqual(expect.objectContaining({
      name: "scout",
      body: "/scout",
      hint: "Map your business systems end to end",
      localOnly: true,
      subscriptionWorkflow: expect.objectContaining({ label: "Scout" }),
    }));
  });

  it("leaves specialist commands visible for the workspace handoff", () => {
    expect(expandPromptSlashCommand("/scout")).toBe("/scout");
    expect(expandPromptSlashCommand("  /scout map AWS")).toBe("  /scout map AWS");
    expect(expandPromptSlashCommand("/scouting")).toBe("/scouting");
    expect(expandPromptSlashCommand("please /scout")).toBe("please /scout");
  });
});

describe("security slash command", () => {
  it("selects the first-class Security space", () => {
    expect(slashCommands()).toContainEqual(expect.objectContaining({
      name: "security",
      body: "/security",
      hint: "Find verified repository vulnerabilities",
      localOnly: true,
      subscriptionWorkflow: expect.objectContaining({ label: "Security Scan" }),
    }));
  });

  it("leaves the exact command prefix for the workspace handoff", () => {
    expect(expandPromptSlashCommand("/security")).toBe("/security");
    expect(expandPromptSlashCommand("  /security crates/auth"))
      .toBe("  /security crates/auth");
    expect(expandPromptSlashCommand("/securityish")).toBe("/securityish");
    expect(expandPromptSlashCommand("please /security")).toBe("please /security");
  });

  it("offers a distinct exact-diff workflow", () => {
    expect(slashCommands()).toContainEqual(expect.objectContaining({
      name: "security-diff",
      body: "/security-diff",
      hint: "Catch security regressions in this diff",
      localOnly: true,
      subscriptionWorkflow: expect.objectContaining({ label: "Security Diff" }),
    }));
    expect(expandPromptSlashCommand("/security-diff"))
      .toBe("/security-diff");
    expect(expandPromptSlashCommand("  /security-diff src"))
      .toBe("  /security-diff src");
    expect(expandPromptSlashCommand("/security-different"))
      .toBe("/security-different");
  });

  it("offers a distinct bounded deep workflow", () => {
    expect(slashCommands()).toContainEqual(expect.objectContaining({
      name: "security-deep",
      body: "/security-deep",
      hint: "Audit deeply with independent passes",
      localOnly: true,
      subscriptionWorkflow: expect.objectContaining({ label: "Security Deep" }),
    }));
    expect(expandPromptSlashCommand("/security-deep"))
      .toBe("/security-deep");
    expect(expandPromptSlashCommand("  /security-deep crates"))
      .toBe("  /security-deep crates");
    expect(expandPromptSlashCommand("/security-deeper"))
      .toBe("/security-deeper");
  });
});

describe("subscription workflow gating", () => {
  it("recognizes slash commands before and after prompt expansion", () => {
    expect(subscriptionWorkflowForSubmission("/scout map AWS")?.command).toBe("scout");
    expect(subscriptionWorkflowForSubmission("$security:security-scan src")?.command)
      .toBe("security");
    expect(subscriptionWorkflowForSubmission("$security:security-diff")?.command)
      .toBe("security-diff");
    expect(subscriptionWorkflowForSubmission("$security:security-deep")?.command)
      .toBe("security-deep");
  });

  it("recognizes a selected premium skill chip", () => {
    expect(subscriptionWorkflowForSubmission(
      "review this",
      ["security:security-deep"],
    )?.label).toBe("Security Deep");
  });

  it("does not gate lookalikes or ordinary skills", () => {
    expect(subscriptionWorkflowForSubmission("/scouting")).toBeNull();
    expect(subscriptionWorkflowForSubmission("$security:security-deeper")).toBeNull();
    expect(subscriptionWorkflowForSubmission("$sentry:sentry")).toBeNull();
  });
});

describe("btw slash command", () => {
  it("stays discoverable before a session and is limited to the local provider", () => {
    const command = slashCommands().find((candidate) => candidate.name === "btw");
    expect(command).toEqual(expect.objectContaining({
      name: "btw",
      body: "/btw",
      localOnly: true,
    }));
    expect(command).not.toHaveProperty("needsSession");
  });

  it("matches only the exact command prefix and extracts its question", () => {
    expect(sideQuestionCommandQuestion("/btw what changed?")).toBe("what changed?");
    expect(sideQuestionCommandQuestion("  /btw   why?  ")).toBe("why?");
    expect(sideQuestionCommandQuestion("/btw")).toBe("");
    expect(sideQuestionCommandQuestion("/btwister")).toBeNull();
    expect(sideQuestionCommandQuestion("please /btw later")).toBeNull();
  });
});
