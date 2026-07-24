import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { Message } from "./Message";

describe("assistant message actions", () => {
  it("places Copy as Markdown in a footer action row after the response", () => {
    const body = "A response with a normal footer action.";
    const markup = renderToStaticMarkup(
      <Message
        role="agent"
        blocks={[{ type: "text", text: body }]}
        timelineIndex={0}
      />,
    );

    expect(markup.indexOf(body)).toBeLessThan(markup.indexOf('aria-label="Copy as Markdown"'));
    expect(markup).toContain("mt-1 flex justify-end");
    expect(markup).not.toContain("absolute -top-1");
  });
});
