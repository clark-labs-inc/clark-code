// Pure derivation of the "Files" view from a snapshot. Aggregates every file a
// tool call touched (by its first location), keeping the most recent tool call
// per path so the viewer shows the latest read/edit. Kept pure so it's unit
// tested without React.

import type { ContentBlock, Snapshot, ToolCall, ToolKind, ToolStatus } from "../../core-bridge/types";

export interface TouchedFile {
  path: string;
  kind: ToolKind;
  status: ToolStatus;
  blocks: ContentBlock[];
  /** Tool call id this view came from (latest touch). */
  toolCall: string;
  /** A diff block renders differently from plain content. */
  isDiff: boolean;
}

function firstPath(tc: ToolCall): string | undefined {
  return tc.locations[0]?.path;
}

function blocksText(blocks: ContentBlock[]): string {
  return blocks.map((b) => (b.type === "text" ? b.text : "")).join("");
}

/**
 * Returns one entry per touched file path. Insertion order follows the order in
 * which the snapshot lists tool calls (i.e. first-seen), but each entry carries
 * the latest tool call that touched the path.
 */
export function deriveTouchedFiles(snapshot: Snapshot): TouchedFile[] {
  const byPath = new Map<string, TouchedFile>();
  const order: string[] = [];

  for (const tc of Object.values(snapshot.tool_calls)) {
    const path = firstPath(tc);
    if (!path) continue;
    if (!byPath.has(path)) order.push(path);
    byPath.set(path, {
      path,
      kind: tc.kind,
      status: tc.status,
      blocks: tc.content,
      toolCall: tc.id,
      isDiff: blocksText(tc.content).startsWith("diff "),
    });
  }

  return order.map((p) => byPath.get(p)!);
}

/** The path the Computer surface should focus, if any. */
export function focusedPath(snapshot: Snapshot): string | undefined {
  return snapshot.focus?.surface === "files" ? snapshot.focus.path ?? undefined : undefined;
}
