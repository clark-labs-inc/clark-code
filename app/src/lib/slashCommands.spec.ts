import { describe, expect, it } from "vitest";
import {
  expandPromptSlashCommand,
  goalCommandObjective,
  isCompactCommand,
  sideQuestionCommandQuestion,
  slashCommands,
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
  it("is discoverable as a local prompt command using the collision-safe skill name", () => {
    expect(slashCommands()).toContainEqual(expect.objectContaining({
      name: "scout",
      body: "$scout:scout",
      hint: "Map a business system with fixed GLM 5.2",
      localOnly: true,
    }));
  });

  it("expands direct command input without matching lookalike commands", () => {
    expect(expandPromptSlashCommand("/scout")).toBe("$scout:scout");
    expect(expandPromptSlashCommand("  /scout map AWS")).toBe("  $scout:scout map AWS");
    expect(expandPromptSlashCommand("/scouting")).toBe("/scouting");
    expect(expandPromptSlashCommand("please /scout")).toBe("please /scout");
  });
});

describe("security slash command", () => {
  it("selects the collision-safe bundled Security workflow", () => {
    expect(slashCommands()).toContainEqual(expect.objectContaining({
      name: "security",
      body: "$security:security-scan",
      hint: "Scan this repository with fixed GLM 5.2",
      localOnly: true,
    }));
  });

  it("expands only the exact command prefix", () => {
    expect(expandPromptSlashCommand("/security")).toBe("$security:security-scan");
    expect(expandPromptSlashCommand("  /security crates/auth"))
      .toBe("  $security:security-scan crates/auth");
    expect(expandPromptSlashCommand("/securityish")).toBe("/securityish");
    expect(expandPromptSlashCommand("please /security")).toBe("please /security");
  });

  it("offers a distinct exact-diff workflow", () => {
    expect(slashCommands()).toContainEqual(expect.objectContaining({
      name: "security-diff",
      body: "$security:security-diff",
      hint: "Review this exact Git diff with fixed GLM 5.2",
      localOnly: true,
    }));
    expect(expandPromptSlashCommand("/security-diff"))
      .toBe("$security:security-diff");
    expect(expandPromptSlashCommand("  /security-diff src"))
      .toBe("  $security:security-diff src");
    expect(expandPromptSlashCommand("/security-different"))
      .toBe("/security-different");
  });

  it("offers a distinct bounded deep workflow", () => {
    expect(slashCommands()).toContainEqual(expect.objectContaining({
      name: "security-deep",
      body: "$security:security-deep",
      hint: "Scan deeply with independent GLM 5.2 passes",
      localOnly: true,
    }));
    expect(expandPromptSlashCommand("/security-deep"))
      .toBe("$security:security-deep");
    expect(expandPromptSlashCommand("  /security-deep crates"))
      .toBe("  $security:security-deep crates");
    expect(expandPromptSlashCommand("/security-deeper"))
      .toBe("/security-deeper");
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
