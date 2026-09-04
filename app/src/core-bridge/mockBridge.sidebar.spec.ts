import { describe, expect, it } from "vitest";

import { MockBridge } from "./mockBridge";

describe("MockBridge sidebar fixture", () => {
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
