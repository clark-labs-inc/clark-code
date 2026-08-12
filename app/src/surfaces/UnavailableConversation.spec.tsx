import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { UnavailableConversationPanel } from "./UnavailableConversation";

describe("UnavailableConversationPanel", () => {
  it("explains the unavailable state and offers retry and cleanup", () => {
    const markup = renderToStaticMarkup(
      <UnavailableConversationPanel
        conversation={{
          id: "missing",
          title: "Review the release",
          detail: "snapshot is unavailable",
          kind: "unavailable",
        }}
        removing={false}
        cleanupError={null}
        allowCleanup
        onRetry={vi.fn()}
        onCleanup={vi.fn()}
      />,
    );

    expect(markup).toContain("Review the release");
    expect(markup).toContain("This chat isn’t available");
    expect(markup).toContain("The chat stays selected");
    expect(markup).toContain("Try again");
    expect(markup).toContain("Clean up");
    expect(markup).toContain("Technical details");
  });

  it("asks for a safe refresh without offering deletion after a cloud conflict", () => {
    const markup = renderToStaticMarkup(
      <UnavailableConversationPanel
        conversation={{
          id: "changed",
          title: "Changed elsewhere",
          detail: "newer cloud revision",
          kind: "refresh_required",
        }}
        removing={false}
        cleanupError={null}
        allowCleanup={false}
        onRetry={vi.fn()}
        onCleanup={vi.fn()}
      />,
    );

    expect(markup).toContain("This chat has a newer version");
    expect(markup).toContain("can’t overwrite newer history");
    expect(markup).toContain("Reload latest");
    expect(markup).not.toContain("Clean up");
  });
});
