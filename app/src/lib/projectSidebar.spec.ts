import { describe, expect, it } from "vitest";
import type { ConversationMeta } from "./history";
import {
  groupSidebarProjects,
  loadProjectSidebarPreferences,
  saveProjectSidebarPreferences,
  withProjectAlias,
  withProjectPinned,
  withoutProjectPreferences,
} from "./projectSidebar";

class MemoryStorage {
  private values = new Map<string, string>();
  getItem(key: string) {
    return this.values.get(key) ?? null;
  }
  setItem(key: string, value: string) {
    this.values.set(key, value);
  }
}

function conversation(id: string, project: string): ConversationMeta {
  return {
    id,
    title: `Chat ${id}`,
    provider: "local",
    project,
    createdAt: 1,
    updatedAt: 1,
  };
}

describe("project sidebar preferences", () => {
  it("persists pins and aliases and clears both when a project is removed", () => {
    const storage = new MemoryStorage();
    let preferences = loadProjectSidebarPreferences(storage);
    preferences = withProjectPinned(preferences, "p:/repo/two", true);
    preferences = withProjectAlias(preferences, "p:/repo/two", "Second repo");
    saveProjectSidebarPreferences(preferences, storage);

    expect(loadProjectSidebarPreferences(storage)).toEqual({
      pinned: ["p:/repo/two"],
      aliases: { "p:/repo/two": "Second repo" },
    });
    expect(withoutProjectPreferences(preferences, "p:/repo/two")).toEqual({
      pinned: [],
      aliases: {},
    });
  });

  it("keeps empty recent folders visible and sorts pinned projects first", () => {
    const conversations = [conversation("one", "/repo/one")];
    const groups = groupSidebarProjects(
      conversations,
      ["/repo/two", "/repo/one"],
      (id) => (id === "one" ? 0 : 1),
      { pinned: ["p:/repo/two"], aliases: { "p:/repo/two": "Second repo" } },
    );

    expect(groups.map((group) => [group.label, group.convos.length])).toEqual([
      ["Second repo", 0],
      ["one", 1],
    ]);
  });
});
