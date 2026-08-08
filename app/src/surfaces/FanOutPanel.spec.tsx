import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { FanOut } from "../core-bridge/types";
import { FanOutCard, fanOutSummary } from "./FanOutPanel";

const fanOut: FanOut = {
  title: "Build, review, and verify the feature",
  total: 3,
  done: 1,
  running: 1,
  agents: [
    { id: "environment", label: "Prepare the test environment", status: "done" },
    { id: "implementation", label: "Implement the change", status: "running" },
    { id: "verification", label: "Review and verify", status: "queued" },
  ],
};

describe("FanOutCard", () => {
  it("renders accessible transcript chips with visible status text", () => {
    const markup = renderToStaticMarkup(<FanOutCard fanOut={fanOut} reduce />);

    expect(markup).toContain("Parallel work");
    expect(markup).toContain("1 running · 1 complete · 1 queued");
    expect(markup).toContain("Prepare the test environment");
    expect(markup).toContain("Implement the change");
    expect(markup).toContain("Complete");
    expect(markup).toContain("Running");
    expect(markup).toContain("Queued");
    expect(markup).toContain("Open subagent details");
  });

  it("includes failures in the aggregate copy", () => {
    expect(
      fanOutSummary({
        ...fanOut,
        agents: fanOut.agents.map((agent, index) =>
          index === 1 ? { ...agent, status: "failed" as const } : agent,
        ),
      }),
    ).toBe("1 complete · 1 queued · 1 needs attention");
    expect(
      fanOutSummary({
        ...fanOut,
        done: 3,
        running: 0,
        agents: fanOut.agents.map((agent) => ({ ...agent, status: "done" as const })),
      }),
    ).toBe("3 complete");
  });
});
