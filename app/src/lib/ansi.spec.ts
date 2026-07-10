import { describe, expect, it } from "vitest";
import { ansiToHtml } from "./ansi";

describe("ansiToHtml", () => {
  it("passes plain text through (escaped, unchanged)", () => {
    expect(ansiToHtml("just text")).toBe("just text");
  });

  it("escapes HTML in plain text", () => {
    expect(ansiToHtml("a < b > c & d")).toBe("a &lt; b &gt; c &amp; d");
  });

  it("converts foreground color codes to themed class spans", () => {
    const out = ansiToHtml("\x1b[31merror\x1b[0m ok \x1b[32mgood\x1b[0m");
    expect(out).toContain('class="ansi-red-fg"');
    expect(out).toContain('class="ansi-green-fg"');
    expect(out).toContain("error");
    expect(out).toContain("good");
  });

  it("handles bright variants and resets", () => {
    const out = ansiToHtml("\x1b[91mhi\x1b[0m");
    expect(out).toContain('class="ansi-bright-red-fg"');
  });

  it("strips codes with no visible color (e.g. cursor moves) without breaking", () => {
    const out = ansiToHtml("\x1b[2K\x1b[1;1Hdone");
    // The non-color escapes are consumed; "done" survives.
    expect(out).toContain("done");
    expect(out).not.toContain("\x1b");
  });
});
