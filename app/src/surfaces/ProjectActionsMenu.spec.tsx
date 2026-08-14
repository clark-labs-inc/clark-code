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
        menuOpen={false}
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
        menuOpen={false}
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
        menuOpen={false}
        reorderable
        onOpenMenu={vi.fn()}
        onNewSession={vi.fn()}
      />,
    );

    expect(html).toContain('data-project-drag-handle="p:/repo"');
    expect(html).toContain("Drag Repo to reorder pinned projects");
    expect(html).toContain("Project actions for Repo");
  });
});
