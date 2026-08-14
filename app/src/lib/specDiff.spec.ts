import { describe, expect, it } from "vitest";

import { specDocumentDiff } from "./specDiff";

describe("specDocumentDiff", () => {
  it("keeps the entire document around a git-like replacement", () => {
    const diff = specDocumentDiff(
      "# Checkout\n\n## Behavior\nCustomers submit the form.\n\n## Done\nStable.",
      "# Checkout\n\n## Behavior\nCustomers review the order before submitting.\n\n## Done\nStable.",
    );

    expect(diff?.added).toBe(1);
    expect(diff?.removed).toBe(1);
    expect(diff?.rows.map((row) => [row.kind, row.text])).toEqual([
      ["equal", "# Checkout"],
      ["equal", ""],
      ["equal", "## Behavior"],
      ["remove", "Customers submit the form."],
      ["add", "Customers review the order before submitting."],
      ["equal", ""],
      ["equal", "## Done"],
      ["equal", "Stable."],
    ]);
  });

  it("counts meaningful additions and removals without counting spacing", () => {
    const diff = specDocumentDiff("# Title\n\nOld", "# Title\n\n\nNew");

    expect(diff?.added).toBe(1);
    expect(diff?.removed).toBe(1);
    expect(diff?.rows.some((row) => row.kind === "add" && row.text === "")).toBe(true);
  });

  it("pairs unrelated replacements so old and new content share the document canvas", () => {
    const diff = specDocumentDiff("# Old\nOld body", "# New\nNew body");

    expect(diff?.rows.map((row) => row.kind)).toEqual([
      "remove",
      "add",
      "remove",
      "add",
    ]);
  });

  it("returns no transition when the saved document did not change", () => {
    expect(specDocumentDiff("same", "same")).toBeNull();
  });
});
