import { describe, expect, it } from "vitest";
import sidebarSource from "./Sidebar.tsx?raw";

describe("Sidebar creation actions", () => {
  it("keeps New session, Quick Chat, and New project as distinct actions", () => {
    expect(sidebarSource).toContain('aria-label="New session"');
    expect(sidebarSource).toContain("New session");
    expect(sidebarSource).toContain('aria-label="Quick Chat"');
    expect(sidebarSource).toContain("> Quick Chat");
    expect(sidebarSource).toContain('aria-label="New project"');
    expect(sidebarSource).toContain("New project…");
    expect(sidebarSource).toContain("onClick={() => newConversation()}");
    expect(sidebarSource).toContain("onClick={() => void startQuickChat()}");
    expect(sidebarSource).toContain("onClick={() => void openProjectTerminal()}");
  });
});
