import { describe, expect, it } from "vitest";
import type { ContentBlock } from "../core-bridge/types";
import { userAttachmentBlocks, userTextBody } from "./messageBlocks";

const text = (t: string): ContentBlock => ({ type: "text", text: t });
const image: ContentBlock = { type: "image", mime_type: "image/webp", data: "aGVsbG8" };
const fileChip: ContentBlock = {
  type: "resource_link",
  uri: "attachment://spec.pdf",
  name: "spec.pdf",
};

describe("userAttachmentBlocks", () => {
  it("returns nothing for a text-only message", () => {
    expect(userAttachmentBlocks([text("hello")])).toEqual([]);
  });

  it("pulls image and file-chip echoes out of a mixed turn", () => {
    expect(userAttachmentBlocks([text("look at these"), image, fileChip])).toEqual([
      image,
      fileChip,
    ]);
  });
});

describe("userTextBody", () => {
  it("joins text blocks", () => {
    expect(userTextBody([text("a"), text("b")])).toBe("ab");
  });

  it("excludes attachments instead of leaking [image] placeholders", () => {
    // The regression this guards: flattening every block put literal
    // `[image]` / `[resource_link]` markers into the copy/edit text.
    const body = userTextBody([text("what is in this screenshot?"), image, fileChip]);
    expect(body).toBe("what is in this screenshot?");
    expect(body).not.toContain("[image]");
    expect(body).not.toContain("spec.pdf");
  });

  it("is empty for an attachment-only message", () => {
    expect(userTextBody([image])).toBe("");
  });
});
