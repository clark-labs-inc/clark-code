import { describe, expect, it } from "vitest";
import { highlight, resolveLang } from "./highlight";

describe("resolveLang", () => {
  it("maps common aliases to a Shiki grammar id", () => {
    expect(resolveLang("ts")).toBe("typescript");
    expect(resolveLang("js")).toBe("javascript");
    expect(resolveLang("sh")).toBe("bash");
    expect(resolveLang("py")).toBe("python");
    expect(resolveLang("rs")).toBe("rust");
    expect(resolveLang("yml")).toBe("yaml");
  });

  it("passes through real Shiki ids", () => {
    expect(resolveLang("rust")).toBe("rust");
    expect(resolveLang("typescript")).toBe("typescript");
  });

  it("drops an info-string's extra tokens (e.g. ```ts title=foo)", () => {
    expect(resolveLang("ts title=foo")).toBe("typescript");
  });

  it("returns null for absent / plainly-text langs", () => {
    expect(resolveLang(undefined)).toBeNull();
    expect(resolveLang("")).toBeNull();
    expect(resolveLang("  ")).toBeNull();
  });
});

describe("highlight", () => {
  it("highlights a known language into Shiki HTML with dual-theme variables", async () => {
    const r = await highlight("const x = 1;", "ts");
    expect(r.lang).toBe("typescript");
    expect(r.html).toBeTruthy();
    // Shiki's dual-theme emits a --shiki-dark variable on tokens.
    expect(r.html).toContain("--shiki-dark");
    // The highlighted output is a <pre> with the shiki class.
    expect(r.html).toContain('class="shiki');
    // And the keyword made it in as a span.
    expect(r.html).toContain("const");
  });

  it("returns null html for an unknown / plain language (plain fallback)", async () => {
    const r = await highlight("just words", "text");
    expect(r.lang).toBe("markdown"); // 'text' aliases to markdown
    const r2 = await highlight("just words", undefined);
    expect(r2.html).toBeNull();
    expect(r2.lang).toBeNull();
  });

  it("loads an on-demand grammar for a language not in the common set", async () => {
    const r = await highlight("println!(\"hi\")", "rust");
    expect(r.lang).toBe("rust");
    // rust IS in the common set, so use a genuinely-uncommon one:
    const r2 = await highlight("x = 1", "ruby");
    expect(r2.lang).toBe("ruby");
    expect(r2.html).toBeTruthy();
  });
});
