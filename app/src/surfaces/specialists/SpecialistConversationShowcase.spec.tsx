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

  it("renders RSI as one compact recursive loop instead of a tabbed dashboard", () => {
    const presentation = specialistConversationPresentation("rsi");
    expect(presentation).not.toBeNull();
    if (!presentation) return;

    const markup = renderToStaticMarkup(
      <SpecialistConversationPresentationCard presentation={presentation} />,
    );

    expect(markup).toContain('data-qa="rsi-loop-pulse"');
    expect(markup).toContain("Measuring the latest change");
    expect(markup).toContain("Best safe result");
    expect(markup).toContain("72.3");
    expect(markup).toContain("Guardrails passing");
    expect(markup).toContain("Production unchanged");
    expect(markup).not.toContain('role="tablist"');
    expect(markup).not.toContain(">Worlds<");
    expect(markup).not.toContain(">Evidence<");
  });
});
