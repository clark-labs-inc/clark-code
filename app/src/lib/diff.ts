import type { ContentBlock, ToolCall } from "../core-bridge/types";

export interface DiffStat {
  adds: number;
  dels: number;
}

/** Flatten a tool result's content blocks to plain text. */
export function blocksToText(blocks: ContentBlock[]): string {
  return blocks.map((b) => (b.type === "text" ? b.text : "")).join("");
}

/** Count added/removed lines in a `diff <path>\n@@…` unified diff (the shape the
 *  local edit/write tools emit). Returns null when the text isn't such a diff. */
export function diffStats(text: string): DiffStat | null {
  if (!text.startsWith("diff ")) return null;
  let adds = 0;
  let dels = 0;
  for (const line of text.split("\n")) {
    if (line.startsWith("diff ") || line.startsWith("@@")) continue;
    if (line.startsWith("+")) adds++;
    else if (line.startsWith("-")) dels++;
  }
  return adds || dels ? { adds, dels } : null;
}

/** Per-edit stats for a single tool call (null if it isn't a rendered edit). */
export function callDiffStat(call: ToolCall): DiffStat | null {
  if (call.kind !== "edit") return null;
  return diffStats(blocksToText(call.content));
}

export interface EditSummary {
  files: number;
  adds: number;
  dels: number;
}

/** Aggregate edit stats across a group of tool calls — the "N files changed,
 *  +X −Y" summary shown under a block of agent work. */
export function summarizeEdits(calls: ToolCall[]): EditSummary | null {
  const byFile = new Map<string, DiffStat>();
  for (const call of calls) {
    const stat = callDiffStat(call);
    if (!stat) continue;
    const path = call.locations[0]?.path ?? call.id;
    const prev = byFile.get(path) ?? { adds: 0, dels: 0 };
    byFile.set(path, { adds: prev.adds + stat.adds, dels: prev.dels + stat.dels });
  }
  if (byFile.size === 0) return null;
  let adds = 0;
  let dels = 0;
  for (const s of byFile.values()) {
    adds += s.adds;
    dels += s.dels;
  }
  return { files: byFile.size, adds, dels };
}
