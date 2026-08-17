import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

import { SPEC_TEXT_REVEAL } from "../../lib/motion";
import { PretextRevealText, specTextRevealInterval } from "./PretextRevealText";

describe("Spec Pretext reveal cadence", () => {
  it("keeps short and long revisions inside the shared motion bounds", () => {
    expect(specTextRevealInterval(1)).toBe(0);
    expect(specTextRevealInterval(4)).toBe(SPEC_TEXT_REVEAL.maxIntervalMs);
    expect(specTextRevealInterval(200)).toBe(SPEC_TEXT_REVEAL.minIntervalMs);
  });

  it("keeps the full accessible text while visual words arrive", () => {
    const markup = renderToStaticMarkup(
      <PretextRevealText text="A complete accessible revision" reduceMotion={false} />,
    );

    expect(markup).toContain('data-pretext-complete="false"');
    expect(markup).toContain("A complete accessible revision");
  });

  it("shows the complete line immediately under reduced motion", () => {
    const markup = renderToStaticMarkup(
      <PretextRevealText text="A calm complete revision" reduceMotion />,
    );

    expect(markup).toContain('data-pretext-complete="true"');
    expect(markup).toContain("A calm complete revision");
  });
});
