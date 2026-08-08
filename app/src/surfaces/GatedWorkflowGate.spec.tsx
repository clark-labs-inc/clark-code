import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { GatedWorkflow } from "../lib/slashCommands";
import { GatedWorkflowGate } from "./GatedWorkflowGate";

const workflow: GatedWorkflow = {
  command: "premium-workflow",
  label: "Premium workflow",
  hint: "Run the product workflow",
  value: "Product-defined workflow value.",
};

function render(covered: boolean, checkingAccess = false) {
  return renderToStaticMarkup(
    <GatedWorkflowGate
      workflow={workflow}
      covered={covered}
      checkingAccess={checkingAccess}
      running={false}
      onRun={vi.fn()}
      onViewAccess={vi.fn()}
      onDismiss={vi.fn()}
    />,
  );
}

describe("GatedWorkflowGate", () => {
  it("explains the workflow value without discarding the Free request", () => {
    const markup = render(false);
    expect(markup).toContain("Premium workflow");
    expect(markup).toContain("Product-defined workflow value");
    expect(markup).toContain("requires product access");
    expect(markup).toContain("Your request is saved.");
    expect(markup).toContain("Review access");
    expect(markup).not.toContain("Run now");
    expect(markup).not.toContain("private-model-name");
  });

  it("runs the saved request without exposing or changing an internal model route", () => {
    const markup = render(true);
    expect(markup).toContain("Access is ready.");
    expect(markup).toContain("Run now");
    expect(markup).not.toContain("Review access");
    expect(markup).not.toContain("private-model-name");
  });

  it("does not offer an action before the access check settles", () => {
    expect(render(false, true)).toContain("Checking access…");
  });
});
