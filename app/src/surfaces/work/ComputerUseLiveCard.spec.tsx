import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { ToolCall } from "../../core-bridge/types";
import { ComputerUseLiveCard } from "./ComputerUseLiveCard";

vi.mock("../../store/sessionStore", () => ({
  useSessionStore: (selector: (state: { cancelActive: () => Promise<void> }) => unknown) =>
    selector({ cancelActive: async () => {} }),
}));

function observation(id: string, title: string, data: string): ToolCall {
  return {
    id,
    tool_name: "computer_get_state",
    title: "View current computer state",
    kind: "view_image",
    status: "completed",
    locations: [],
    content: [
      { type: "text", text: `Window: Safari — "${title}"\nObservation ID: ${id}` },
      { type: "image", mime_type: "image/png", data },
    ],
  };
}

describe("ComputerUseLiveCard", () => {
  it("renders a stacked live computer state with app context and controls", () => {
    const markup = renderToStaticMarkup(
      createElement(ComputerUseLiveCard, {
        calls: [observation("observe-1", "Checkout", "QUJD"), observation("observe-2", "Confirmation", "REVG")],
      }),
    );

    expect(markup).toContain("Computer use");
    expect(markup).toContain("Safari · Confirmation");
    expect(markup).not.toContain("Take over");
    expect(markup).toContain("Stop the agent computer use");
    expect(markup).toContain("data:image/png;base64,REVG");
    expect(markup).toContain("Show computer screenshot history");
  });

  it("does not render when a computer tool has no screenshot yet", () => {
    const markup = renderToStaticMarkup(
      createElement(ComputerUseLiveCard, {
        calls: [{
          id: "windows-1",
          tool_name: "computer_list_windows",
          title: "List windows",
          kind: "search",
          status: "completed",
          locations: [],
          content: [{ type: "text", text: "No matching windows." }],
        }],
      }),
    );

    expect(markup).toBe("");
  });

  it("exposes move and resize affordances in floating mode", () => {
    const markup = renderToStaticMarkup(
      createElement(ComputerUseLiveCard, {
        calls: [observation("observe-1", "Checkout", "QUJD")],
        floating: true,
      }),
    );

    expect(markup).toContain("Drag to move this panel");
    expect(markup).toContain("Resize computer use panel");
    expect(markup).toContain("absolute");
  });

  it("collapses identical observations instead of stacking the same screenshot", () => {
    const markup = renderToStaticMarkup(
      createElement(ComputerUseLiveCard, {
        calls: [observation("observe-1", "Checkout", "QUJD"), observation("observe-2", "Checkout", "QUJD")],
      }),
    );

    expect(markup.match(/Safari current computer state/g)).toHaveLength(1);
    expect(markup).not.toContain("Safari previous computer state");
  });
});
