import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import {
  initialMacPermissionStep,
  MacPermissionGuide,
} from "./MacPermissionGuide";

describe("MacPermissionGuide", () => {
  it("starts on the first missing permission", () => {
    expect(initialMacPermissionStep(false, false)).toBe("accessibility");
    expect(initialMacPermissionStep(true, false)).toBe("screen-recording");
    expect(initialMacPermissionStep(true, true)).toBe("accessibility");
  });

  it("explains the separate Clark Computer Use identity and both setup steps", () => {
    const markup = renderToStaticMarkup(
      <MacPermissionGuide
        ownerName="Clark Computer Use"
        accessibilityGranted={false}
        screenRecordingGranted={false}
        working={false}
        onRequestPermissions={vi.fn()}
      />,
    );
    expect(markup).toContain("Let’s set up Clark on your Mac");
    expect(markup).toContain("Clark Computer Use");
    expect(markup).toContain("Accessibility");
    expect(markup).toContain("Screen Recording");
    expect(markup).toContain("Open macOS settings");
    expect(markup).toContain('aria-label="Show Allow Screen Recording"');
  });

  it("marks already-granted access in the visual walkthrough", () => {
    const markup = renderToStaticMarkup(
      <MacPermissionGuide
        ownerName="Clark Computer Use Dev"
        accessibilityGranted
        screenRecordingGranted={false}
        working={false}
        onRequestPermissions={vi.fn()}
      />,
    );
    expect(markup).toContain("Clark Computer Use Dev");
    expect(markup).toContain('aria-current="step"');
    expect(markup).toContain("2 / 2");
  });
});
