import { describe, expect, it } from "vitest";
import type { ContentBlock } from "../core-bridge/types";
import { contentText, imageBlocks, imageSource, sameContentBlocks } from "./contentBlocks";

const text = (value: string): ContentBlock => ({ type: "text", text: value });
const image = (data = "aGVsbG8"): ContentBlock => ({
  type: "image",
  mime_type: "image/png",
  data,
});

describe("typed content blocks", () => {
  it("keeps image attachments out of a text-only tool detail", () => {
    const blocks = [text("Viewed design.png\n"), image(), text("Image is 1x1.")];

    expect(contentText(blocks)).toBe("Viewed design.png\nImage is 1x1.");
    expect(contentText(blocks)).not.toContain("[image]");
    expect(imageBlocks(blocks)).toHaveLength(1);
  });

  it("builds a safe image source from typed inline bytes or a remote URI", () => {
    expect(imageSource(imageBlocks([image("cGl4ZWxz")])[0])).toBe(
      "data:image/png;base64,cGl4ZWxz",
    );
    expect(
      imageSource({ type: "image", mime_type: "image/png", data: "", uri: "https://cdn.example/image.png" }),
    ).toBe("https://cdn.example/image.png");
    expect(
      imageSource({ type: "image", mime_type: "image/png", data: "", uri: "/private/image.png" }),
    ).toBeNull();
  });

  it("detects a changed image payload even when the block count is unchanged", () => {
    expect(sameContentBlocks([text("result"), image("first")], [text("result"), image("second")])).toBe(false);
    expect(sameContentBlocks([text("result"), image("first")], [text("result"), image("first")])).toBe(true);
  });
});
