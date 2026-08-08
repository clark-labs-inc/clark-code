// Serialize a conversation snapshot to clean Markdown — "Copy conversation".

import type { Snapshot } from "../core-bridge/types";

/** The visible conversation as Markdown: messages verbatim, tool work as a
 *  compact bullet list, artifacts by title. */
export function conversationMarkdown(snapshot: Snapshot): string {
  const out: string[] = [];
  let work: string[] = [];
  const flushWork = () => {
    if (work.length === 0) return;
    out.push(work.map((w) => `- ${w}`).join("\n"));
    work = [];
  };
  for (const item of snapshot.timeline) {
    if (item.item === "tool_call") {
      const call = snapshot.tool_calls[item.id];
      if (call?.title) work.push(call.title);
      continue;
    }
    flushWork();
    if (item.item === "message") {
      const text = item.blocks
        .map((b) =>
          b.type === "text"
            ? b.text
            : b.type === "skill_reference"
              ? `[$${b.name}]`
              : `[${b.type}]`,
        )
        .join("")
        .trim();
      if (!text) continue;
      out.push(item.role === "user" ? `**You:**\n${text}` : text);
    } else if (item.item === "artifact") {
      const a = snapshot.artifacts.find((x) => x.id === item.id);
      if (a) out.push(`> Artifact: ${a.title ?? a.id}`);
    }
  }
  flushWork();
  return out.join("\n\n");
}
