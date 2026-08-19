import { describe, expect, it, vi } from "vitest";

import { resolveSpecSelectionSkillReferences } from "./SpecSelectionThread";

const specCatalog = {
  skills: [{
    id: "spec-skill",
    revision: "spec-revision",
    invocationName: "spec:spec",
    enabled: true,
  }],
};

describe("selection-scoped Spec submission", () => {
  it("reloads a stale catalog before declaring the section discussion unavailable", async () => {
    const list = vi.fn(async () => ({ skills: [] }));
    const reload = vi.fn(async () => specCatalog);

    await expect(resolveSpecSelectionSkillReferences(list, reload)).resolves.toEqual([{
      type: "skill_reference",
      id: "spec-skill",
      revision: "spec-revision",
      name: "spec:spec",
    }]);
    expect(list).toHaveBeenCalledOnce();
    expect(reload).toHaveBeenCalledOnce();
  });

  it("uses reload as recovery when the background catalog read fails", async () => {
    const list = vi.fn(async () => { throw new Error("catalog not ready"); });
    const reload = vi.fn(async () => specCatalog);

    await expect(resolveSpecSelectionSkillReferences(list, reload)).resolves.toHaveLength(1);
    expect(reload).toHaveBeenCalledOnce();
  });

  it("does not reload a catalog that already contains the Spec workflow", async () => {
    const list = vi.fn(async () => specCatalog);
    const reload = vi.fn(async () => specCatalog);

    await expect(resolveSpecSelectionSkillReferences(list, reload)).resolves.toHaveLength(1);
    expect(reload).not.toHaveBeenCalled();
  });
});
