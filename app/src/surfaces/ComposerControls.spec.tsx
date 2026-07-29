import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ModelPriceCue } from "./ComposerControls";

describe("ModelPriceCue", () => {
  it("renders no price cue for the included model", () => {
    expect(renderToStaticMarkup(<ModelPriceCue tier={0} />)).toBe("");
  });

  it("renders relative price tiers one through five as tiny decorative dollar signs", () => {
    for (const tier of [1, 2, 3, 4, 5] as const) {
      const markup = renderToStaticMarkup(<ModelPriceCue tier={tier} />);

      expect(markup).toContain(`>${"$".repeat(tier)}</span>`);
      expect(markup).toContain('aria-hidden="true"');
      expect(markup).toContain("text-xs");
      expect(markup).toContain("opacity-60");
    }
  });
});
