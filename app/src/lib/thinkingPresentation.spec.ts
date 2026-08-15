import { describe, expect, it } from "vitest";
import { thinkingForDisplay } from "./thinkingPresentation";

describe("thinkingForDisplay", () => {
  it("repairs provider token newlines without changing the source value", () => {
    const raw =
      "The\n check\n is\n running\n i\nn the\n backg\nro\nund. Let\n me wait fo\nr i\nt. I'\nll\n u\nse bash\n_w\nait w\nith a reasonable\n t\nimeout for the dep\n ende\nnc\nies to compi\nle.";

    expect(thinkingForDisplay(raw)).toBe(
      "The check is running in the background. Let me wait for it. I'll use bash_wait with a reasonable timeout for the dependencies to compile.",
    );
    expect(raw).toContain("backg\nro\nund");
  });

  it("leaves ordinary Markdown and short prose untouched", () => {
    const markdown = "First paragraph.\n\n- one\n- two\n\n`inline`";
    expect(thinkingForDisplay(markdown)).toBe(markdown);
    expect(thinkingForDisplay("line one\nline two")).toBe("line one\nline two");
  });

  it("preserves Markdown structure and fenced code in a token-wrapped block", () => {
    const raw =
      "I\n'll ins\npect this.\n- Keep this item\n- Keep that item\n\n```text\na\nb\n```\nThen cont\ninue\n now.";

    const displayed = thinkingForDisplay(raw);
    expect(displayed).toContain("I'll inspect this.\n- Keep this item\n- Keep that item");
    expect(displayed).toContain("```text\na\nb\n```");
    expect(displayed).toContain("Then continue now.");
  });
});
