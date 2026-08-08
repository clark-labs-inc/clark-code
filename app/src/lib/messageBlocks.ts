// User-message block partitioning. The host echoes a sent turn as text blocks
// plus one block per attachment (image with inline bytes / resource_link chip)
// — this splits those back apart so the bubble renders thumbnails and chips
// and the copy/edit text stays free of "[image]"-style placeholders.

import type { ContentBlock } from "../core-bridge/types";

/** Blocks a user message shows as attachment thumbnails/chips, not text. */
export function userAttachmentBlocks(blocks: ContentBlock[]): ContentBlock[] {
  return blocks.filter(
    (b) => b.type === "image"
      || b.type === "audio"
      || b.type === "resource"
      || b.type === "resource_link"
      || b.type === "skill_reference",
  );
}

/** The text body of a user message — attachment blocks excluded. */
export function userTextBody(blocks: ContentBlock[]): string {
  return blocks
    .filter((b): b is { type: "text"; text: string } => b.type === "text")
    .map((b) => b.text)
    .join("");
}
