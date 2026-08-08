import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import {
  SpecialistConversationPresentationCard,
  SpecialistConversationShowcase,
} from "./SpecialistConversationShowcase";
import { specialistConversationPresentation } from "../../lib/specialistPresentation";

describe("SpecialistConversationShowcase", () => {
  it("renders an honest conversation example with compact rich presentation controls", () => {
    const markup = renderToStaticMarkup(
      <SpecialistConversationShowcase kind="security" onUsePrompt={vi.fn()} />,
    );

    expect(markup).toContain("Illustrative example");
    expect(markup).toContain("Demo data");
    expect(markup).toContain("Archive extraction can cross the workspace boundary");
    expect(markup).toContain("Decision signal");
    expect(markup).toContain('role="progressbar"');
    expect(markup).toContain('role="tablist"');
    expect(markup).toContain(">Map<");
    expect(markup).toContain(">Evidence<");
    expect(markup).toContain(">Run<");
    expect(markup).toContain("Use prompt");
  });

  it("renders the same presentation as an inline conversation result", () => {
    const presentation = specialistConversationPresentation("security");
    expect(presentation).not.toBeNull();
    if (!presentation) return;

    const markup = renderToStaticMarkup(
      <SpecialistConversationPresentationCard presentation={presentation} />,
    );

    expect(markup).toContain("Specialist analysis");
    expect(markup).toContain("Evidence and decision surface");
    expect(markup).not.toContain("Use prompt");
    expect(markup).toContain("Validated attack path");
  });
});
