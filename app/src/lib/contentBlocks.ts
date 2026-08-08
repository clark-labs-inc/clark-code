import type { ContentBlock } from "../core-bridge/types";

export type ImageContentBlock = Extract<ContentBlock, { type: "image" }>;

/** Text that belongs in a tool/message body, excluding typed attachments. */
export function contentText(blocks: ContentBlock[]): string {
  return blocks
    .filter((block): block is Extract<ContentBlock, { type: "text" }> => block.type === "text")
    .map((block) => block.text)
    .join("");
}

/** Typed image outputs, kept separate from the text used in tool details. */
export function imageBlocks(blocks: ContentBlock[]): ImageContentBlock[] {
  return blocks.filter((block): block is ImageContentBlock => block.type === "image");
}

/** A browser-safe source for a typed image block, when it has one. */
export function imageSource(image: ImageContentBlock): string | null {
  if (image.data) return `data:${image.mime_type};base64,${image.data}`;
  const uri = image.uri?.trim();
  if (!uri || (!/^data:image\//i.test(uri) && !/^https?:\/\//i.test(uri))) return null;
  return uri;
}

/** Field-wise equality avoids reparsing/rerendering unchanged large image data. */
export function sameContentBlocks(a: ContentBlock[], b: ContentBlock[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((block, index) => {
    const other = b[index];
    if (block.type !== other.type) return false;
    if (block.type === "text" || block.type === "thinking") {
      return block.text === (other as typeof block).text;
    }
    if (block.type === "image") {
      const image = other as ImageContentBlock;
      return block.mime_type === image.mime_type && block.data === image.data && block.uri === image.uri;
    }
    if (block.type === "audio") {
      const audio = other as Extract<ContentBlock, { type: "audio" }>;
      return block.mime_type === audio.mime_type && block.data === audio.data;
    }
    if (block.type === "resource") {
      const resource = other as Extract<ContentBlock, { type: "resource" }>;
      return block.uri === resource.uri
        && block.mime_type === resource.mime_type
        && block.text === resource.text
        && block.data === resource.data;
    }
    if (block.type === "skill_reference") {
      const skill = other as Extract<ContentBlock, { type: "skill_reference" }>;
      return block.id === skill.id && block.revision === skill.revision && block.name === skill.name;
    }
    const link = other as Extract<ContentBlock, { type: "resource_link" }>;
    return block.uri === link.uri && block.name === link.name;
  });
}
