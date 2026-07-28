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
});
