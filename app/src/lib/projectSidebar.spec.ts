import { describe, expect, it } from "vitest";
import type { ConversationMeta } from "./history";
import {
  groupSidebarProjects,
  isQuickChatProject,
  loadProjectSidebarPreferences,
  projectDisplayName,
  projectDisplayTitle,
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

function remoteConversation(id: string, host: string, project: string): ConversationMeta {
  return {
    ...conversation(id, project),
    remoteHost: host,
  };
}

describe("project sidebar preferences", () => {
  it("keeps managed checkouts visibly tied to their source repository", () => {
    const path = "/repo/clark-desktop.clark-worktrees/session-1";
    expect(projectDisplayName(path)).toBe("session-1 · clark-desktop");
    expect(projectDisplayTitle(path)).toContain("Isolated checkout of clark-desktop");
    expect(
      groupSidebarProjects(
        [conversation("managed", path)],
        [],
        () => 0,
        { pinned: [], aliases: {} },
      )[0]?.label,
    ).toBe("session-1");
    expect(
      groupSidebarProjects(
        [conversation("managed", path)],
        [],
        () => 0,
        { pinned: [], aliases: {} },
      )[0]?.repositoryLabel,
    ).toBe("clark-desktop");
  });

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

  it("keeps sidebar preferences separate between accounts", () => {
    const storage = new MemoryStorage();
    saveProjectSidebarPreferences(
      { pinned: ["p:/previous"], aliases: { "p:/previous": "Previous" } },
      storage,
      "id:previous",
    );

    expect(loadProjectSidebarPreferences(storage, "id:new")).toEqual({
      pinned: [],
      aliases: {},
    });
    expect(loadProjectSidebarPreferences(storage, "id:previous")).toEqual({
      pinned: ["p:/previous"],
      aliases: { "p:/previous": "Previous" },
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

  it("keeps the SSH destination on aliased remote groups", () => {
    const groups = groupSidebarProjects(
      [remoteConversation("remote", "ubuntu@cpu", "/home/ubuntu/clark")],
      [],
      () => 0,
      { pinned: [], aliases: { "r:ubuntu@cpu": "Build server" } },
    );

    expect(groups[0]).toMatchObject({
      kind: "remote",
      label: "Build server",
      remoteHost: "ubuntu@cpu",
      remoteRoot: "/home/ubuntu/clark",
    });
  });

  it("groups app-managed workspaces as Quick chats instead of projects", () => {
    const id = "912a9700-7f5f-4f18-9785-b5d9315a41b4";
    const path = `/Users/alex/.clark/workspace/${id}`;
    const groups = groupSidebarProjects(
      [conversation(id, path)],
      [],
      () => 0,
      { pinned: [], aliases: {} },
    );

    expect(isQuickChatProject(path, id)).toBe(true);
    expect(projectDisplayName(path)).toBe("Quick Chat");
    expect(projectDisplayTitle(path)).toBe("Temporary Clark workspace");
    expect(groups).toHaveLength(1);
    expect(groups[0]).toMatchObject({
      key: "quick-chats",
      kind: "none",
      label: "Quick chats",
    });
    expect(isQuickChatProject(`/home/another-user/.clark/workspace/${id}`, id)).toBe(true);
    expect(isQuickChatProject(`/Users/alex/.clark/workspace/different-id`, id)).toBe(false);
  });
});
