import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { ProjectGroup } from "../lib/projectSidebar";
import { ProjectHeader } from "./ProjectActionsMenu";

function group(overrides: Partial<ProjectGroup>): ProjectGroup {
  return {
    key: "none",
    label: "Other",
    title: "Other",
    kind: "none",
    convos: [],
    latest: 0,
    ...overrides,
  };
}

describe("ProjectHeader", () => {
  it("offers a new session directly on remote project rows", () => {
    const html = renderToStaticMarkup(
      <ProjectHeader
        group={group({
          key: "r:ubuntu@cpu",
          label: "ubuntu@cpu",
          title: "Remote · ubuntu@cpu · /home/ubuntu/project",
          kind: "remote",
          remoteHost: "ubuntu@cpu",
        })}
        expanded
        conversationPanelId="project-conversations-remote"
        menuOpen={false}
        onToggle={vi.fn()}
        onOpenMenu={vi.fn()}
        onNewSession={vi.fn()}
      />,
    );

    expect(html).toContain('title="New session on ubuntu@cpu"');
    expect(html).toContain('aria-label="New session on ubuntu@cpu"');
    expect(html.indexOf("New session on ubuntu@cpu")).toBeLessThan(
      html.indexOf("Project actions for ubuntu@cpu"),
    );
  });

  it("does not offer a new session for the unscoped Other group", () => {
    const html = renderToStaticMarkup(
      <ProjectHeader
        group={group({})}
        expanded={false}
        conversationPanelId="project-conversations-other"
        menuOpen={false}
        onToggle={vi.fn()}
        onOpenMenu={vi.fn()}
        onNewSession={vi.fn()}
      />,
    );

    expect(html).not.toContain("New session");
  });

  it("shows an explicit drag handle only for reorderable pinned projects", () => {
    const html = renderToStaticMarkup(
      <ProjectHeader
        group={group({ key: "p:/repo", label: "Repo", kind: "local", path: "/repo" })}
        expanded={false}
        conversationPanelId="project-conversations-repo"
        menuOpen={false}
        reorderable
        onToggle={vi.fn()}
        onOpenMenu={vi.fn()}
        onNewSession={vi.fn()}
      />,
    );

    expect(html).toContain('data-project-drag-handle="p:/repo"');
    expect(html).toContain("Drag Repo to reorder pinned projects");
    expect(html).toContain("Project actions for Repo");
  });

  it("exposes project expansion and the conversation count from the header", () => {
    const html = renderToStaticMarkup(
      <ProjectHeader
        group={group({
          key: "quick-chats",
          label: "Quick chats",
          convos: [
            { id: "one", title: "One", provider: "local", createdAt: 1, updatedAt: 1 },
            { id: "two", title: "Two", provider: "local", createdAt: 2, updatedAt: 2 },
          ],
        })}
        expanded
        conversationPanelId="project-conversations-quick"
        menuOpen={false}
        onToggle={vi.fn()}
        onOpenMenu={vi.fn()}
        onNewSession={vi.fn()}
      />,
    );

    expect(html).toContain('aria-label="Collapse Quick chats"');
    expect(html).toContain('aria-controls="project-conversations-quick"');
    expect(html).toContain(">2</span>");
  });
});
