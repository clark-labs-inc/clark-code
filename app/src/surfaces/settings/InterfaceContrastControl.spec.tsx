import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { InterfaceContrastControl } from "./InterfaceContrastControl";

describe("interface contrast", () => {
  it("starts at medium and exposes an extra-high option", () => {
    const markup = renderToStaticMarkup(
      <InterfaceContrastControl value="medium" onChange={() => {}} />,
    );

    expect(markup).toContain("Overall contrast");
    expect(markup).toContain("across the whole interface");
    expect(markup).toMatch(/aria-pressed="true"[^>]*>Medium<\/button>/);
    expect(markup).toContain(">Extra high</button>");
  });
});
