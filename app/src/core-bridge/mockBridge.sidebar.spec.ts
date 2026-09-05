import { describe, expect, it } from "vitest";

import { MockBridge } from "./mockBridge";
import { groupSidebarProjects, isQuickChatProject } from "../lib/projectSidebar";

describe("MockBridge sidebar fixture", () => {
  it("groups newly created quick chats together instead of creating project rows", async () => {
    const bridge = new MockBridge();
    const first = await bridge.prepareQuickChatWorkspace();
    const second = await bridge.prepareQuickChatWorkspace();
    expect(first.id).not.toBe(second.id);
    expect(isQuickChatProject(first.path, first.id)).toBe(true);
    const conversations = [first, second].map((workspace) => ({
      id: workspace.id, project: workspace.path, title: "New Quick Chat",
      provider: "local", createdAt: 1, updatedAt: 1,
    }));
    const groups = groupSidebarProjects(conversations, [], () => 0, { pinned: [], aliases: {} });
    expect(groups).toHaveLength(1);
    expect(groups[0].key).toBe("quick-chats");
    expect(groups[0].convos).toHaveLength(2);
  });
  it("creates distinct conversation ids for a realistic multi-conversation list", async () => {
    const bridge = new MockBridge();

    const first = await bridge.openSession("local", {}, { kind: "new", options: {} });
    const second = await bridge.openSession("local", {}, { kind: "new", options: {} });
    const third = await bridge.openSession("local", {}, { kind: "new", options: {} });

    expect(first.id).toBe("mock-session");
    expect(new Set([first.id, second.id, third.id]).size).toBe(3);
  });

  it("keeps browser-preview quick chats recognizable after they reopen", async () => {
    const bridge = new MockBridge();
    const workspace = await bridge.prepareQuickChatWorkspace(
      "00000000-0000-4000-8000-000000000001",
    );

    expect(workspace.path).toBe(
      "/mock/.agent/workspace/00000000-0000-4000-8000-000000000001",
    );
  });
});
