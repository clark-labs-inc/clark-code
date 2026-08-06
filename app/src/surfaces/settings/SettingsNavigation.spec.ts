import { describe, expect, it } from "vitest";
import { filterSettingsGroups } from "./SettingsNavigation";

function ids(query: string) {
  return filterSettingsGroups(query).flatMap((group) => group.items.map((item) => item.id));
}

describe("settings navigation search", () => {
  it("returns every section for an empty query", () => {
    expect(ids("")).toEqual([
      "general",
      "account",
      "project",
      "commands",
      "integrations",
      "computer-use",
      "about",
    ]);
  });

  it("matches section content keywords", () => {
    expect(ids("font")).toEqual(["general"]);
    expect(ids("ssh")).toEqual(["integrations"]);
    expect(ids("billing")).toEqual(["account"]);
    expect(ids("accessibility")).toEqual(["computer-use"]);
  });

  it("returns no groups when nothing matches", () => {
    expect(ids("not-a-real-setting")).toEqual([]);
  });
});
