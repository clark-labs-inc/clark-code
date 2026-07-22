import { describe, expect, it } from "vitest";
import {
  expandPromptSlashCommand,
  goalCommandObjective,
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
