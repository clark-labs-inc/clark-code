import { LazyMotion, domMax } from "motion/react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { SpecDocumentDiff } from "./SpecDocumentDiff";

describe("SpecDocumentDiff", () => {
  it("renders the whole document as an in-place revision instead of a floating receipt", () => {
    const markup = renderToStaticMarkup(
      <LazyMotion features={domMax} strict>
        <SpecDocumentDiff diff={{
          revision: 2,
          added: 1,
          removed: 1,
          rows: [
            { kind: "equal", text: "# Checkout", previousLine: 1, nextLine: 1 },
            { kind: "equal", text: "", previousLine: 2, nextLine: 2 },
            { kind: "equal", text: "## Interaction rules", previousLine: 3, nextLine: 3 },
            { kind: "remove", text: "Users can select text.", previousLine: 4, nextLine: null },
            { kind: "add", text: "Agent edits temporarily lock selection.", previousLine: null, nextLine: 4 },
            { kind: "equal", text: "## Acceptance criteria", previousLine: 5, nextLine: 5 },
          ],
        }} />
      </LazyMotion>,
    );

    expect(markup).toContain('data-qa="spec-document-diff"');
    expect(markup).toContain("Checkout");
    expect(markup).toContain("Interaction rules");
    expect(markup).toContain("Users can select text.");
    expect(markup).toContain("Agent edits temporarily lock selection.");
    expect(markup).toContain("Acceptance criteria");
    expect(markup).toContain('data-diff-kind="remove"');
    expect(markup).toContain('data-diff-kind="add"');
    expect(markup).toContain('data-qa="spec-pretext-reveal"');
    expect(markup).toContain('data-pretext-complete="false"');
    expect(markup).not.toContain("Editing specification");
  });
});
