import { describe, expect, it } from "vitest";
import { goalCommandObjective, slashCommands } from "./slashCommands";

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
