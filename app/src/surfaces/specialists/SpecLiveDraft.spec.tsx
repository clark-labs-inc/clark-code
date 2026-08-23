import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { SpecLiveDraft as Draft } from "../../lib/specLiveDraft";
import { SpecLiveDraft } from "./SpecLiveDraft";

function draft(over: Partial<Draft> = {}): Draft {
  return { kind: "document", text: "", settled: false, callId: "w", ...over };
}

describe("SpecLiveDraft", () => {
  it("renders settled lines as real Markdown and the open line separately", () => {
    const markup = renderToStaticMarkup(
      <SpecLiveDraft draft={draft({ text: "# Slime environments\n\n## Recomm" })} />,
    );

    // The closed line became a heading; the open one is still plain text held by
    // the Pretext-measured block.
    expect(markup).toContain("<h1");
    expect(markup).toContain("Slime environments");
    expect(markup).toContain('data-qa="spec-streaming-line"');
    expect(markup).toContain("## Recomm");
  });

  it("does not announce every streamed token to a screen reader", () => {
    const markup = renderToStaticMarkup(
      <SpecLiveDraft draft={draft({ text: "# Spec\n\nBody text here" })} />,
    );

    expect(markup).toContain('aria-live="polite"');
    expect(markup).toContain("Writing the specification.");
    // The transient line is decoration; the settled Markdown above is the copy.
    expect(markup).toMatch(/aria-hidden="true"[^>]*data-qa="spec-streaming-line"/);
  });

  it("frames a fragment as an incoming revision rather than the document", () => {
    const markup = renderToStaticMarkup(
      <SpecLiveDraft draft={draft({ kind: "revision", text: "new wording\n", path: "new_SPEC.md" })} />,
    );

    expect(markup).toContain("Writing a revision");
    expect(markup).toContain("new_SPEC.md");
    expect(markup).toContain('data-draft-kind="revision"');
  });

  it("types a revision's live line in the same compact mono as its settled text", () => {
    const markup = renderToStaticMarkup(
      <SpecLiveDraft draft={draft({ kind: "revision", text: "settled words\nstill typ" })} />,
    );

    // The panel sets mono/text-xs for the settled fragment; the line being
    // typed must match or each keystroke lands a size larger than its context.
    expect(markup).toMatch(/data-qa="spec-streaming-line"[^>]*class="[^"]*font-mono/);
    expect(markup).not.toMatch(/data-qa="spec-streaming-line"[^>]*class="[^"]*text-sm/);
  });

  it("gives a list line being typed the bullet its settled neighbours have", () => {
    const markup = renderToStaticMarkup(
      <SpecLiveDraft draft={draft({ text: "- done item\n- being typ" })} />,
    );

    expect(markup).toContain("being typ");
    // The raw dash is syntax; the bullet is what the line is becoming.
    expect(markup).toContain("\u2022");
    expect(markup).not.toContain("- being typ");
  });

  it("renders a live line inside an open fence as code, not a bullet", () => {
    const markup = renderToStaticMarkup(
      <SpecLiveDraft draft={draft({ text: "Steps:\n\n```diff\n- old value" })} />,
    );

    // `- old value` here is diff syntax. Bulletizing it would misread the
    // document, and the type must be monospace from the first frame.
    expect(markup).toContain("- old value");
    expect(markup).not.toContain("\u2022");
    expect(markup).toMatch(/font-mono[^"]*"[^>]*>(?:(?!<\/div>).)*- old value/s);
  });

  it("keeps heading syntax visible while a section arrives", () => {
    // `## ` tells the reader a section is coming; hiding it would be a loss.
    const markup = renderToStaticMarkup(
      <SpecLiveDraft draft={draft({ text: "Body.\n\n## Safety bound" })} />,
    );

    expect(markup).toContain("## Safety bound");
  });

  it("renders an unmeasured first frame as flowed text rather than nothing", () => {
    // Server render has no layout, so geometry is never ready there.
    const markup = renderToStaticMarkup(<SpecLiveDraft draft={draft({ text: "First words" })} />);

    expect(markup).toContain("First words");
    expect(markup).not.toContain("data-pretext-lines");
  });
});
