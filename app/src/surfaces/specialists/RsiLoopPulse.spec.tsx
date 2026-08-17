import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { specialistConversationPresentation } from "../../lib/specialistPresentation";
import { RsiLoopPulseCard, rsiLoopStages } from "./RsiLoopPulse";

describe("RsiLoopPulseCard", () => {
  it("normalizes the universal five-stage recursive improvement loop", () => {
    const presentation = specialistConversationPresentation("rsi");
    expect(presentation).not.toBeNull();
    if (!presentation) return;

    const stages = rsiLoopStages(presentation);

    expect(stages.map((stage) => stage.label)).toEqual([
      "Inspect",
      "Propose",
      "Code",
      "Measure",
      "Decide",
    ]);
    expect(stages.map((stage) => stage.status)).toEqual([
      "complete",
      "complete",
      "complete",
      "active",
      "queued",
    ]);
  });

  it("keeps an accessible DOM loop when WebGL has not loaded", () => {
    const presentation = specialistConversationPresentation("rsi");
    expect(presentation).not.toBeNull();
    if (!presentation) return;

    const markup = renderToStaticMarkup(
      <RsiLoopPulseCard presentation={presentation} />,
    );

    expect(markup).toContain('aria-label="RSI recursive improvement loop"');
    expect(markup).toContain('aria-label="RSI loop stages"');
    expect(markup).toContain("Current stage: Measure");
    expect(markup).toContain('aria-label="Show RSI loop details"');
  });
});
