import { describe, expect, it } from "vitest";
import sidebarSource from "./Sidebar.tsx?raw";
import headerSource from "./sidebar/SidebarHeader.tsx?raw";
// The row itself lives in its own memoized module; the sidebar wires the state
// into it. Assertions about row markup have to read the row's source.
import conversationRowSource from "./sidebar/ConversationRow.tsx?raw";

describe("Sidebar creation actions", () => {
  it("uses one folder chooser and a separate project-free quick chat action", () => {
    expect(headerSource).toContain('aria-label="New session"');
    expect(headerSource).toContain('onClick={() => chooseProject(true)}');
    expect(headerSource).toContain('aria-label="New quick chat"');
    expect(headerSource).toContain("await quickChat()");
    expect(headerSource).not.toContain('aria-label="New project"');
  });

  it("renders a blue finished-not-yet-visited dot from unseen open rows", () => {
    // The row state is fed from the store's `unseenWorkIds`, and the dot is
    // rendered in the same leading slot as the streaming/pulsing indicator.
    expect(sidebarSource).toContain("const unseenWorkIds = useSessionStore(");
    expect(sidebarSource).toContain("unseen={");
    expect(sidebarSource).toContain("unseenWorkIds.includes(c.id)");
    expect(conversationRowSource).toContain("bg-info");
    // A stream in flight outranks the finished marker on the same row.
    const streamingIndex = conversationRowSource.indexOf(") : streaming ?");
    const unseenIndex = conversationRowSource.indexOf(") : unseen ?");
    expect(streamingIndex).toBeGreaterThan(-1);
    expect(unseenIndex).toBeGreaterThan(streamingIndex);
  });

  it("scrolls specialist navigation together with ordinary conversations", () => {
    const scrollRegion = sidebarSource.indexOf('ref={conversationListRef}');
    const specialistNavigation = sidebarSource.indexOf('<SpecialistNavigation />', scrollRegion);

    expect(scrollRegion).toBeGreaterThan(-1);
    expect(specialistNavigation).toBeGreaterThan(scrollRegion);
  });
});
