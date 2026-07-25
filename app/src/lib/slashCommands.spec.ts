import { describe, expect, it } from "vitest";
import {
  expandPromptSlashCommand,
  goalCommandObjective,
  isCompactCommand,
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
