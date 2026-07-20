import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ActivityRewardToast } from "./ActivityRewardToast";
import { useSessionStore } from "../store/sessionStore";

describe("ActivityRewardToast", () => {
  it("labels a server-issued bonus reward as work earned", () => {
    const previous = useSessionStore.getState().activityReward;
    useSessionStore.setState({
      activityReward: { id: "reward-1", credits: 450, tier: "bonus", createdAt: "2026-07-19T12:00:00Z" },
    });

    const markup = renderToStaticMarkup(<ActivityRewardToast />);
    useSessionStore.setState({ activityReward: previous });

    expect(markup).toContain("Bonus reward");
    expect(markup).toContain("Your work earned +450 credits.");
  });
});
