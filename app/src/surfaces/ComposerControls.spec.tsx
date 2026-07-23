import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { projectedRunUsage, UsageChipView } from "./ComposerControls";

describe("UsageChip", () => {
  it("prefers cumulative usage while a run is still active", () => {
    expect(projectedRunUsage({
      id: "run-1",
      status: "running",
      usage: {
        input_tokens: 1_000,
        output_tokens: 100,
        context_tokens: 1_000,
        cost_usd: 0.01,
        context_limit: 10_000,
      },
    })?.cost_usd).toBe(0.01);
  });

  it("keeps terminal-only snapshots compatible", () => {
    expect(projectedRunUsage({
      id: "run-1",
      status: "done",
      outcome: {
        status: "done",
        usage: {
          input_tokens: 2_000,
          output_tokens: 200,
          context_tokens: 2_000,
          context_limit: 10_000,
        },
      },
    })?.context_tokens).toBe(2_000);
  });

  it("shows only the percentage of the context limit used", () => {
    const markup = renderToStaticMarkup(
      <UsageChipView contextTokens={75_000} contextLimit={300_000} />,
    );

    expect(markup).toContain("25% of limit used");
    expect(markup).not.toContain("$");
    expect(markup).not.toContain("75,000");
    expect(markup).not.toContain("tokens");
  });
});
