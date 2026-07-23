import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ActivityRewardReceipt } from "./ActivityRewardToast";

describe("ActivityRewardToast", () => {
  it("labels a server-issued bonus reward as work earned", () => {
    const markup = renderToStaticMarkup(
      <ActivityRewardReceipt
        reward={{ id: "reward-1", credits: 450, tier: "bonus", createdAt: "2026-07-19T12:00:00Z" }}
        onDismiss={() => {}}
      />,
    );

    expect(markup).toContain("Bonus reward");
    expect(markup).toContain("Your work earned an activity reward.");
    expect(markup).not.toContain("450");
    expect(markup).not.toContain("credits");
  });
});
