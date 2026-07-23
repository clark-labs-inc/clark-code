import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { SkillCatalogSnapshot } from "../core-bridge/bridge";
import { SkillsPanel } from "./SkillsPanel";

const catalog: SkillCatalogSnapshot = {
  revision: "catalog_1234567890",
  environmentId: "local:/workspace",
  projectRoot: "/workspace",
  skills: [
    {
      id: "skill_project",
      revision: "skill_revision_1",
      name: "review",
      invocationName: "project:compatible:review",
      description: "Review the selected change.",
      scope: "project",
      origin: "compatible",
      source: "/workspace/.agents/skills/review/SKILL.md",
      requiredTools: [],
      missingTools: [],
      allowImplicitInvocation: true,
      enabled: true,
      disabledReason: null,
      hasNameCollision: true,
    },
    {
      id: "skill_user",
      revision: "skill_revision_2",
      name: "review",
      invocationName: "user:claude:review",
      description: "Run the personal review workflow.",
      scope: "user",
      origin: "claude",
      source: "/home/user/.claude/skills/review/SKILL.md",
      requiredTools: ["browser"],
      missingTools: ["browser"],
      allowImplicitInvocation: false,
      enabled: false,
      disabledReason: "Requires browser in this environment.",
      hasNameCollision: true,
    },
  ],
  diagnostics: [
    {
      severity: "error",
      code: "invalid_skill",
      message: "A skill has invalid metadata.",
      source: "/workspace/.agents/skills/broken/SKILL.md",
    },
  ],
};

describe("SkillsPanel", () => {
  it("surfaces exact collision-safe bindings, health, and environment-scoped packs", () => {
    const markup = renderToStaticMarkup(
      <SkillsPanel
        open
        bridge={null}
        cwd="/workspace"
        remote={null}
        catalog={catalog}
        loading={false}
        error={null}
        onClose={vi.fn()}
        onReload={vi.fn(async () => catalog)}
        onCatalog={vi.fn()}
        onSelect={vi.fn()}
      />,
    );

    expect(markup).toContain("$project:compatible:review");
    expect(markup).toContain("$user:claude:review");
    expect(markup).toContain("collision preserved");
    expect(markup).toContain("Requires browser in this environment.");
    expect(markup).toContain("error · invalid_skill");
    expect(markup).toContain("No instruction files discovered.");
    expect(markup).toContain("This environment");
  });
});
