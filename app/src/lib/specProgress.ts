import type { Snapshot, ToolCall } from "../core-bridge/types";

/** Strip provider/tool protocol prefixes from copy shown to a person. */
export function specProgressTitle(call: Pick<ToolCall, "title">): string {
  return call.title.replace(/^[a-z][a-z0-9_]*:\s*/i, "").trim() || "Working";
}

/** Tool calls for only the latest user turn, in durable timeline order. This
 * keeps a reopened Spec from presenting old work as current live progress. */
export function currentSpecToolCalls(snapshot: Snapshot): ToolCall[] {
  let latestUserIndex = -1;
  for (let index = snapshot.timeline.length - 1; index >= 0; index -= 1) {
    const item = snapshot.timeline[index];
    if (item.item === "message" && item.role === "user") {
      latestUserIndex = index;
      break;
    }
  }

  const seen = new Set<string>();
  const calls: ToolCall[] = [];
  for (const item of snapshot.timeline.slice(latestUserIndex + 1)) {
    if (item.item !== "tool_call" || seen.has(item.id)) continue;
    const call = snapshot.tool_calls[item.id];
    if (!call) continue;
    seen.add(item.id);
    calls.push(call);
  }
  return calls;
}
