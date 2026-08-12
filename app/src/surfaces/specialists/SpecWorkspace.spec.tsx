import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { SpecWritingSkeleton } from "./SpecWorkspace";

describe("SpecWritingSkeleton", () => {
  it("renders a semantic repository-focused working state with animated writing lines", () => {
    const markup = renderToStaticMarkup(<SpecWritingSkeleton repositoryFocused />);

    expect(markup).toContain('role="status"');
    expect(markup).toContain("Clark is reading the focused repository and writing the specification");
    expect(markup).toContain("Reading the focused code and shaping the next section");
    expect(markup.match(/spec-writing-line/g)).toHaveLength(4);
  });
});
