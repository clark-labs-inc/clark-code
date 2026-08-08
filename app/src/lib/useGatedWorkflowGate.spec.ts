import { describe, expect, it } from "vitest";
import type { GatedWorkflow } from "./slashCommands";
import { gatedWorkflowNeedsAccessGate } from "./useGatedWorkflowGate";

const gated: GatedWorkflow = {
  command: "premium-workflow",
  label: "Premium workflow",
  hint: "Run the product workflow",
  value: "Product-defined value",
};

describe("product workflow access gate", () => {
  it("depends on authoritative coverage rather than the visible model picker", () => {
    expect(gatedWorkflowNeedsAccessGate(gated, false, true)).toBe(false);
    expect(gatedWorkflowNeedsAccessGate(gated, false, false)).toBe(true);
  });

  it("does not gate ordinary requests or an approved retry", () => {
    expect(gatedWorkflowNeedsAccessGate(null, false, false)).toBe(false);
    expect(gatedWorkflowNeedsAccessGate(gated, true, false)).toBe(false);
  });
});
