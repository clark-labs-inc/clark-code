import { describe, expect, it } from "vitest";
import { SUBSCRIPTION_WORKFLOWS } from "./slashCommands";
import { subscriptionWorkflowNeedsCoverageGate } from "./useSubscriptionWorkflowGate";

const scout = SUBSCRIPTION_WORKFLOWS.find((workflow) => workflow.command === "scout")!;

describe("subscriber workflow coverage gate", () => {
  it("depends on authoritative coverage rather than the visible model picker", () => {
    expect(subscriptionWorkflowNeedsCoverageGate(scout, false, true)).toBe(false);
    expect(subscriptionWorkflowNeedsCoverageGate(scout, false, false)).toBe(true);
  });

  it("does not gate ordinary requests or an approved retry", () => {
    expect(subscriptionWorkflowNeedsCoverageGate(null, false, false)).toBe(false);
    expect(subscriptionWorkflowNeedsCoverageGate(scout, true, false)).toBe(false);
  });
});
