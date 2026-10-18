import { describe, expect, it } from "vitest";
import type { ConversationMeta } from "./history";
import {
  groupSidebarProjects,
  isQuickChatProject,
  loadProjectSidebarPreferences,
  projectDisplayName,
  projectDisplayTitle,
  remoteProjectKey,
  saveProjectSidebarPreferences,
  withProjectAlias,
  withProjectPinned,
  withPinnedProjectMoved,
  withProjectRemoved,
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
    const path = "/repo/example-desktop.agent-worktrees/session-1";
    expect(projectDisplayName(path)).toBe("session-1 · example-desktop");
    expect(projectDisplayTitle(path)).toContain("Isolated checkout of example-desktop");
    expect(
      groupSidebarProjects(
        [conversation("managed", path)],
        [],
        () => 0,
        { removed: [], pinned: [], aliases: {} },
      )[0]?.label,
    ).toBe("session-1");
    expect(
      groupSidebarProjects(
        [conversation("managed", path)],
        [],
        () => 0,
        { removed: [], pinned: [], aliases: {} },
      )[0]?.repositoryLabel,
    ).toBe("example-desktop");
  });

  it("keeps legacy Clark-managed checkouts tied to their source repository", () => {
    const path = "/repo/clark-twitter.clark-worktrees/clark-twitter-main-84d96c71";
    expect(projectDisplayName(path)).toBe("clark-twitter-main-84d96c71 · clark-twitter");
    expect(projectDisplayTitle(path)).toContain("Isolated checkout of clark-twitter");
  });

  it("persists pins and aliases and unpins a removed project", () => {
    const storage = new MemoryStorage();
    let preferences = loadProjectSidebarPreferences(storage);
    preferences = withProjectPinned(preferences, "p:/repo/two", true);
    preferences = withProjectAlias(preferences, "p:/repo/two", "Second repo");
    saveProjectSidebarPreferences(preferences, storage);

    expect(loadProjectSidebarPreferences(storage)).toEqual({
      removed: [],
      pinned: ["p:/repo/two"],
      aliases: { "p:/repo/two": "Second repo" },
    });
    expect(withProjectRemoved(preferences, "p:/repo/two", true)).toEqual({
      removed: ["p:/repo/two"],
      pinned: [],
      aliases: { "p:/repo/two": "Second repo" },
    });
  });

  it("uses the same exact pinned-project movement for drag and menu actions", () => {
    const preferences = {
      removed: [],
      pinned: ["p:/repo/one", "p:/repo/two", "p:/repo/three"],
      aliases: { "p:/repo/two": "Second repo" },
    };

    expect(withPinnedProjectMoved(preferences, "p:/repo/three", 0)).toEqual({
      removed: [],
      pinned: ["p:/repo/three", "p:/repo/one", "p:/repo/two"],
      aliases: preferences.aliases,
    });
    expect(withPinnedProjectMoved(preferences, "p:/repo/one", 99).pinned).toEqual([
      "p:/repo/two",
      "p:/repo/three",
      "p:/repo/one",
    ]);
    expect(withPinnedProjectMoved(preferences, "p:/repo/missing", 1)).toBe(preferences);
  });

  it("keeps sidebar preferences separate between accounts", () => {
    const storage = new MemoryStorage();
    saveProjectSidebarPreferences(
      { removed: [], pinned: ["p:/previous"], aliases: { "p:/previous": "Previous" } },
      storage,
      "id:previous",
    );

    expect(loadProjectSidebarPreferences(storage, "id:new")).toEqual({
      removed: [],
      pinned: [],
      aliases: {},
    });
    expect(loadProjectSidebarPreferences(storage, "id:previous")).toEqual({
      removed: [],
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
      { removed: [], pinned: ["p:/repo/two"], aliases: { "p:/repo/two": "Second repo" } },
    );

    expect(groups.map((group) => [group.label, group.convos.length])).toEqual([
      ["Second repo", 0],
      ["one", 1],
    ]);
  });

  it("keeps the SSH destination on aliased remote groups", () => {
    const key = remoteProjectKey("ubuntu@cpu", "/home/ubuntu/project");
    const groups = groupSidebarProjects(
      [remoteConversation("remote", "ubuntu@cpu", "/home/ubuntu/project")],
      [],
      () => 0,
      { removed: [], pinned: [], aliases: { [key]: "Build server" } },
    );

    expect(groups[0]).toMatchObject({
      kind: "remote",
      label: "Build server",
      remoteHost: "ubuntu@cpu",
      remoteRoot: "/home/ubuntu/project",
    });
  });

  it("keeps different repositories on the same SSH host as different projects", () => {
    const groups = groupSidebarProjects(
      [
        remoteConversation("api", "ubuntu@cpu", "/srv/api"),
        remoteConversation("web", "ubuntu@cpu", "/srv/web"),
      ],
      [],
      (id) => id === "api" ? 0 : 1,
      { removed: [], pinned: [], aliases: {} },
    );

    expect(groups).toHaveLength(2);
    expect(groups.map((group) => [group.label, group.remoteRoot, group.convos[0]?.id])).toEqual([
      ["api", "/srv/api", "api"],
      ["web", "/srv/web", "web"],
    ]);
  });

  it("groups app-managed workspaces as Quick chats instead of projects", () => {
    const id = "912a9700-7f5f-4f18-9785-b5d9315a41b4";
    const path = `/Users/alex/.agent/workspace/${id}`;
    const groups = groupSidebarProjects(
      [conversation(id, path)],
      [],
      () => 0,
      { removed: [], pinned: [], aliases: {} },
    );

    expect(isQuickChatProject(path, id)).toBe(true);
    expect(projectDisplayName(path)).toBe("Quick Chat");
    expect(projectDisplayTitle(path)).toBe("Temporary the agent workspace");
    expect(groups).toHaveLength(1);
    expect(groups[0]).toMatchObject({
      key: "quick-chats",
      kind: "none",
      label: "Quick chats",
    });
    expect(isQuickChatProject(`/home/another-user/.agent/workspace/${id}`, id)).toBe(true);
    expect(isQuickChatProject(`/Users/alex/.agent/workspace/different-id`, id)).toBe(false);
  });
  it("keeps removal after reload even when unarchived cloud chats and remembered folders return", () => {
    const storage = new MemoryStorage();
    const chats = [conversation("one", "/repo/one")];
    const removed = withProjectRemoved(loadProjectSidebarPreferences(storage, "id:one"), "p:/repo/one", true);
    saveProjectSidebarPreferences(removed, storage, "id:one");
    const restored = loadProjectSidebarPreferences(storage, "id:one");

    expect(groupSidebarProjects(chats, ["/repo/one"], () => 0, restored)).toEqual([]);
    expect(chats[0].archived).toBeUndefined();
    expect(groupSidebarProjects(chats, [], () => 0, loadProjectSidebarPreferences(storage, "id:two"))).toHaveLength(1);
    expect(groupSidebarProjects(chats, [], () => 0, restored, "one")[0].convos).toEqual(chats);
    expect(groupSidebarProjects(chats, [], () => 0, withProjectRemoved(restored, "p:/repo/one", false))).toHaveLength(1);
  });

  it("removes only the selected remote root and preserves aliases for recovery", () => {
    const chats = [remoteConversation("api", "cpu", "/srv/api"), remoteConversation("web", "cpu", "/srv/web")];
    const key = remoteProjectKey("cpu", "/srv/api");
    const initial = withProjectAlias(loadProjectSidebarPreferences(), key, "Backend");
    const removed = withProjectRemoved(initial, key, true);
    expect(groupSidebarProjects(chats, [], () => 0, removed).map((g) => g.label)).toEqual(["web"]);
    expect(groupSidebarProjects(chats, [], () => 0, removed, "Backend").map((g) => g.label)).toContain("Backend");
  });

  it("migrates existing preferences without a removal list and rejects malformed entries", () => {
    const storage = new MemoryStorage();
    storage.setItem("agent-desktop:project-sidebar", JSON.stringify({ pinned: [], aliases: {}, removed: [null, 3, "p:/repo"] }));
    expect(loadProjectSidebarPreferences(storage).removed).toEqual(["p:/repo"]);
    storage.setItem("agent-desktop:project-sidebar", JSON.stringify({ pinned: ["p:/repo"], aliases: {} }));
    expect(loadProjectSidebarPreferences(storage)).toEqual({ pinned: ["p:/repo"], aliases: {}, removed: [] });
  });

});
