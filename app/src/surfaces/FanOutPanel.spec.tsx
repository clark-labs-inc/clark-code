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
  it("keeps agent details collapsed by default", () => {
    const markup = renderToStaticMarkup(<FanOutCard fanOut={fanOut} reduce />);

    expect(markup).toContain("Parallel work");
    expect(markup).toContain("1 of 3 parts ready");
    expect(markup).toContain('aria-expanded="false"');
    expect(markup).not.toContain("Prepare the test environment");
  });

  it("prioritizes failures and completion in the aggregate copy", () => {
    expect(
      fanOutSummary({
        ...fanOut,
        agents: fanOut.agents.map((agent, index) =>
          index === 1 ? { ...agent, status: "failed" as const } : agent,
        ),
      }),
    ).toBe("1 part needs attention");
    expect(fanOutSummary({ ...fanOut, done: 3, running: 0 })).toBe("All parts are ready");
  });
});
