import { describe, expect, it } from "vitest";
import { createElement, Fragment } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { privacySafePanelReference } from "./PanelErrorBoundary";
import { PanelErrorBoundary } from "./PanelErrorBoundary";

describe("privacySafePanelReference", () => {
  it("returns a stable opaque reference rather than raw runtime details", () => {
    const error = new Error("provider=private token=secret tool_call=shell:89");

    const first = privacySafePanelReference(error, "at Conversation");
    const second = privacySafePanelReference(error, "at Conversation");

    expect(first).toMatch(/^DESKTOP-[0-9A-F]{8}$/);
    expect(second).toBe(first);
    expect(first).not.toContain("secret");
    expect(first).not.toContain("shell:89");
  });

  it("renders a contained fallback while adjacent workspace content survives", () => {
    const boundary = new PanelErrorBoundary({
      title: "Conversation panel restarted",
      children: createElement("div", null, "broken panel"),
    });
    boundary.state = {
      error: new Error("token=secret provider=private"),
      reference: "DESKTOP-1234ABCD",
    };

    const html = renderToStaticMarkup(
      createElement(
        Fragment,
        null,
        createElement("aside", null, "workspace navigation survives"),
        boundary.render(),
      ),
    );

    expect(html).toContain("workspace navigation survives");
    expect(html).toContain("Conversation panel restarted");
    expect(html).toContain("DESKTOP-1234ABCD");
    expect(html).not.toContain("token=secret");
    expect(html).not.toContain("provider=private");
  });
});
