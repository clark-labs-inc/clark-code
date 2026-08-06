import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { SUBSCRIPTION_WORKFLOWS } from "../lib/slashCommands";
import { SubscriptionWorkflowGate } from "./SubscriptionWorkflowGate";

const workflow = SUBSCRIPTION_WORKFLOWS.find(
  (candidate) => candidate.command === "security-deep",
)!;

function render(covered: boolean, checkingCoverage = false) {
  return renderToStaticMarkup(
    <SubscriptionWorkflowGate
      workflow={workflow}
      covered={covered}
      checkingCoverage={checkingCoverage}
      running={false}
      onRun={vi.fn()}
      onViewPlans={vi.fn()}
      onDismiss={vi.fn()}
    />,
  );
}

describe("SubscriptionWorkflowGate", () => {
  it("explains the workflow value without discarding the Free request", () => {
    const markup = render(false);
    expect(markup).toContain("Security Deep");
    expect(markup).toContain("Run independent security passes");
    expect(markup).toContain("A Clark subscription unlocks");
    expect(markup).toContain("Your request is saved.");
    expect(markup).toContain("View plans");
    expect(markup).not.toContain("Run now");
    expect(markup).not.toContain("DeepSeek");
  });

  it("runs the saved request without exposing or changing an internal model route", () => {
    const markup = render(true);
    expect(markup).toContain("Your Clark coverage is ready.");
    expect(markup).toContain("Run now");
    expect(markup).not.toContain("View plans");
    expect(markup).not.toContain("DeepSeek");
  });

  it("does not offer plans before the billing check settles", () => {
    expect(render(false, true)).toContain("Checking your plan…");
  });
});
