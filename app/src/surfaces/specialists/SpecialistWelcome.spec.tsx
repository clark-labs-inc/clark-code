import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { SpecialistWelcome, specialistStarters } from "./SpecialistWelcome";

describe("SpecialistWelcome", () => {
  it("leads with workspace guidance and keeps the illustrative analysis opt-in", () => {
    const markup = renderToStaticMarkup(
      <SpecialistWelcome kind="security" onStart={vi.fn()} />,
    );

    expect(markup).toContain("Choose a repository-level investigation");
    expect(markup).not.toContain("Security workspace");
    expect(markup).toContain("Choose a starting point");
    expect(markup).toContain("Example analysis");
    expect(markup).toContain("Nothing runs until you send it");
    expect(markup).not.toContain("Illustrative example");
    expect(markup).not.toContain("Clark Security");
    expect(markup).not.toContain("Try:");
  });

  it("maps each Security starting point to the right specialist workflow", () => {
    expect(specialistStarters("security").map(({ tab, workflow }) => ({ tab, workflow }))).toEqual([
      { tab: "scans", workflow: "security:security-deep" },
      { tab: "scans", workflow: "security:security-diff" },
      { tab: "posture", workflow: "security:security-scan" },
    ]);
  });
});
