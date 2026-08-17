import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import {
  ScoutFullAccessIndicator,
  SpecialistFullAccessIndicator,
} from "./ComposerPermissionPill";

describe("ComposerPermissionPill", () => {
  it("renders Scout Full access as a fixed indicator, not a selector", () => {
    const markup = renderToStaticMarkup(<ScoutFullAccessIndicator />);

    expect(markup).toContain("Scout uses protected Full access");
    expect(markup).toContain("Full access");
    expect(markup).not.toContain("aria-haspopup");
    expect(markup).not.toContain("Ask for approval");
  });

  it("makes Spec's non-negotiable boundaries visible", () => {
    const markup = renderToStaticMarkup(<SpecialistFullAccessIndicator specialist="spec" />);

    expect(markup).toContain("Spec uses protected Full access");
    expect(markup).toContain("Full access · protected");
    expect(markup).toContain("File deletion and GitHub pushes stay blocked");
    expect(markup).not.toContain("aria-haspopup");
  });
});
