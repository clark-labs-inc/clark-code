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
    expect(sidebarSource).toContain("onClick={() => setNewProjectOpen(true)}");
  });

  it("renders a blue finished-not-yet-visited dot from unseen open rows", () => {
    // The row state is fed from the store's `unseenWorkIds`, and the dot is
    // rendered in the same leading slot as the streaming/pulsing indicator.
    expect(sidebarSource).toContain("const unseenWorkIds = useSessionStore(");
    expect(sidebarSource).toContain("unseen={");
    expect(sidebarSource).toContain("unseenWorkIds.includes(c.id)");
    expect(sidebarSource).toContain("bg-info");
    // A stream in flight outranks the finished marker on the same row.
    const streamingIndex = sidebarSource.indexOf(") : streaming ?");
    const unseenIndex = sidebarSource.indexOf(") : unseen ?");
    expect(streamingIndex).toBeGreaterThan(-1);
    expect(unseenIndex).toBeGreaterThan(streamingIndex);
  });
});
