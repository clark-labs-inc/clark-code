import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { ComposerDraftConflict } from "./ComposerDraftConflict";

describe("ComposerDraftConflict", () => {
  it("frames a genuine conflict as an actionable draft choice", () => {
    const markup = renderToStaticMarkup(
      <ComposerDraftConflict onKeepCurrent={vi.fn()} onUseSynced={vi.fn()} />,
    );

    expect(markup).toContain('role="group"');
    expect(markup).toContain('aria-label="Choose which draft to keep"');
    expect(markup).toContain("Another saved draft is available.");
    expect(markup).toContain("Keep current draft");
    expect(markup).toContain("Use saved draft");
    expect(markup).not.toContain("cloud sync");
    expect(markup).not.toContain("paused");
  });
});
