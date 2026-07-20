import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { activityAge, ParallelWorkContext } from "./ParallelWorkContext";

describe("ParallelWorkContext", () => {
  it("summarizes Clark and Codex peers without overstating detection", () => {
    const html = renderToStaticMarkup(
      <ParallelWorkContext
        branch="main"
        clarkPeers={[{ id: "clark-1", title: "Run the frontend tests" }]}
        activity={{
          changedFiles: 2,
          untrackedFiles: 1,
          conflictedFiles: 0,
          externalAgents: [{
            id: "codex-1",
            title: "Fix the composer",
            agentNickname: null,
            updatedAtMs: 90_000,
          }],
          detectedAtMs: 100_000,
        }}
      />,
    );

    expect(html).toContain("2 agents");
    expect(html).toContain("2 other agents active in this checkout");
    expect(html).not.toContain("Parallel work details");
  });

  it("uses bounded, plain-language recency labels", () => {
    expect(activityAge(80_000, 100_000)).toBe("active now");
    expect(activityAge(0, 120_000)).toBe("2m ago");
  });
});
