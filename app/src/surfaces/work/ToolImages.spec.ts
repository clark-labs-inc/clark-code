import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { ContentBlock } from "../../core-bridge/types";
import { ToolImages } from "./WorkLine";

describe("ToolImages", () => {
  it("renders a typed tool image as an inline data URL", () => {
    const blocks: ContentBlock[] = [
      { type: "text", text: "Viewed mockup.png." },
      { type: "image", mime_type: "image/png", data: "QUJD" },
    ];

    const markup = renderToStaticMarkup(createElement(ToolImages, { blocks }));

    expect(markup).toContain('src="data:image/png;base64,QUJD"');
    expect(markup).not.toContain("[image]");
  });
});
