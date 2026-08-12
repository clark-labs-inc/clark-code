import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { Message } from "./Message";

describe("assistant message actions", () => {
  it("overlays Copy as Markdown without adding a footer row", () => {
    const body = "A response with a normal footer action.";
    const markup = renderToStaticMarkup(
      <Message
        role="agent"
        blocks={[{ type: "text", text: body }]}
        timelineIndex={0}
      />,
    );

    expect(markup).toContain("group/msg relative");
    expect(markup).toContain("absolute -top-1 right-0");
    expect(markup).not.toContain("mt-1 flex justify-end");
  });

  it("renders assistant output through the streaming-aware markdown layer", () => {
    const markup = renderToStaticMarkup(
      <Message
        role="agent"
        blocks={[{ type: "text", text: "A **streaming** response." }]}
        timelineIndex={0}
        streaming
      />,
    );

    expect(markup).toContain('data-sd-animate="true"');
    expect(markup).toContain("<strong><span");
    expect(markup).toContain(">streaming</span></strong>");
    expect(markup).not.toContain("--streamdown-caret");
  });

  it("keeps every assistant markdown layer on the full message rail", () => {
    const markup = renderToStaticMarkup(
      <Message
        role="agent"
        blocks={[{ type: "text", text: "First paragraph.\n\nSecond paragraph that must not collapse." }]}
        timelineIndex={0}
        streaming
      />,
    );

    expect(markup).toContain("min-w-0 w-full space-y-1.5");
    expect(markup).toContain("min-w-0 w-full text-base");
    expect(markup).toMatch(/space-y-4 whitespace-normal[^\"]*min-w-0 w-full/);
  });

  it("uses the shared semantic text palette", () => {
    const markup = renderToStaticMarkup(
      <Message
        role="agent"
        blocks={[{ type: "text", text: "A calmer response." }]}
        timelineIndex={0}
      />,
    );

    expect(markup).toContain("text-ink");
  });

  it("marks new user and assistant rows with role-specific entrance motion", () => {
    const user = renderToStaticMarkup(
      <Message
        role="user"
        blocks={[{ type: "text", text: "Please animate this row." }]}
        timelineIndex={1}
        animateEntry
      />,
    );
    const assistant = renderToStaticMarkup(
      <Message
        role="agent"
        blocks={[{ type: "text", text: "This answer arrives progressively." }]}
        timelineIndex={2}
        animateEntry
      />,
    );

    expect(user).toContain('data-chat-message-role="user"');
    expect(user).toContain('data-chat-message-motion="enter"');
    expect(assistant).toContain('data-chat-message-role="assistant"');
    expect(assistant).toContain('data-chat-message-motion="enter"');
    expect(assistant).toContain('data-sd-animate="true"');
  });
});
