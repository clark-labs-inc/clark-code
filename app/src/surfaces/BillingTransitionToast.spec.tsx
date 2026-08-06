import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { BillingTransitionReceipt } from "./BillingTransitionToast";

describe("BillingTransitionReceipt", () => {
  it("celebrates newly available subscriber workflows", () => {
    const markup = renderToStaticMarkup(
      <BillingTransitionReceipt
        transition={{
          id: 1,
          kind: "upgraded",
          title: "Your Clark subscription is ready",
          detail: "Scout, Security, and subscriber workflows are now available.",
          tier: "paid",
        }}
        onDismiss={vi.fn()}
        onViewBilling={vi.fn()}
      />,
    );
    expect(markup).toContain("Your Clark subscription is ready");
    expect(markup).toContain("Scout, Security, and subscriber workflows are now available.");
    expect(markup).not.toContain("DeepSeek");
    expect(markup).not.toContain("Review billing");
  });

  it("reassures users when coverage ends and offers the relevant action", () => {
    const markup = renderToStaticMarkup(
      <BillingTransitionReceipt
        transition={{
          id: 2,
          kind: "downgraded",
          title: "Free is now active",
          detail: "Your drafts and conversations are safe.",
          tier: "free",
        }}
        onDismiss={vi.fn()}
        onViewBilling={vi.fn()}
      />,
    );
    expect(markup).toContain("Free is now active");
    expect(markup).toContain("Your drafts and conversations are safe.");
    expect(markup).toContain("Review billing");
  });
});
