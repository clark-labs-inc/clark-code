import { describe, expect, it } from "vitest";
import { paletteCommandPresentation } from "./commandPalette";

describe("paletteCommandPresentation", () => {
  it("keeps prompt commands visibly named, searchable, and editable", () => {
    const item = paletteCommandPresentation({
      name: "btw",
      hint: "Ask a side question without interrupting the run",
      body: "/btw",
    });

    expect(item.label).toBe("/btw");
    expect(item.hint).toBe("Ask a side question without interrupting the run");
    expect(item.searchText).toContain("btw");
    expect(item.prefill).toBe("/btw ");
  });

  it("keeps action commands executable rather than turning them into prefills", () => {
    const item = paletteCommandPresentation(
      { name: "new", hint: "Start a new conversation", run: () => {} },
      "New session",
    );

    expect(item.label).toBe("New session");
    expect(item.hint).toBe("Start a new conversation");
    expect(item.prefill).toBeNull();
  });
});
