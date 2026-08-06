import type { ContentBlock, Snapshot, ToolCall } from "./types";

const RESERVED_PROTOCOL_MARKERS = [
  "begin_of_sentence",
  "require_escalated_model",
  "expiration_placeholder",
  "skillconstraint_hard",
] as const;

function containsReservedProtocolMarker(value: string): boolean {
  const normalized = value.toLowerCase().replace(/_+/g, "_");
  return RESERVED_PROTOCOL_MARKERS.some((marker) => normalized.includes(marker));
}

function blocksAreContaminated(blocks: ContentBlock[]): boolean {
  return blocks.some((block) => {
    if (block.type === "text" || block.type === "thinking") {
      return containsReservedProtocolMarker(block.text);
    }
    return block.type === "resource"
      && typeof block.text === "string"
      && containsReservedProtocolMarker(block.text);
  });
}

function structuredValueIsContaminated(root: unknown): boolean {
  const pending: unknown[] = [root];
  let inspected = 0;
  while (pending.length > 0 && inspected < 10_000) {
    inspected += 1;
    const value = pending.pop();
    if (typeof value === "string") {
      if (containsReservedProtocolMarker(value)) return true;
    } else if (Array.isArray(value)) {
      pending.push(...value);
    } else if (value !== null && typeof value === "object") {
      pending.push(...Object.values(value));
    }
  }
  // An object too large to validate is not safe to replay into model context.
  return pending.length > 0;
}

function toolCallIsContaminated(toolCall: ToolCall): boolean {
  return blocksAreContaminated(toolCall.content)
    || structuredValueIsContaminated(toolCall.raw_input);
}

function checkpointIsContaminated(snapshot: Snapshot): boolean {
  return snapshot.model_context_checkpoint?.transcript.items.some((item) => {
    if (item.item === "message") {
      return item.role === "agent" && blocksAreContaminated(item.blocks);
    }
    if (item.item === "tool_call") {
      return blocksAreContaminated(item.content)
        || structuredValueIsContaminated(item.arguments);
    }
    return false;
  }) ?? false;
}

/**
 * Remove provider protocol residue before a snapshot reaches rendering or a
 * resumed model context. The complete provider turn is the trust boundary: a
 * marker in any agent block invalidates that message, and a marker in a tool
 * call invalidates that call. User-authored messages are preserved.
 */
export function quarantineSnapshotProviderOutput(snapshot: Snapshot): Snapshot {
  const rejectedTools = new Set(
    Object.entries(snapshot.tool_calls)
      .filter(([, toolCall]) => toolCallIsContaminated(toolCall))
      .map(([id]) => id),
  );
  let contaminated = rejectedTools.size > 0 || checkpointIsContaminated(snapshot);
  const timeline = snapshot.timeline.filter((item) => {
    const reject = (item.item === "message"
        && item.role === "agent"
        && blocksAreContaminated(item.blocks))
      || (item.item === "tool_call" && rejectedTools.has(item.id));
    contaminated ||= reject;
    return !reject;
  });
  if (!contaminated) return snapshot;

  const toolCalls = Object.fromEntries(
    Object.entries(snapshot.tool_calls).filter(([id]) => !rejectedTools.has(id)),
  );
  return {
    ...snapshot,
    timeline,
    tool_calls: toolCalls,
    // A checkpoint can replay hidden reasoning/tool arguments that the visible
    // projection no longer contains. Rebuild it from the sanitized transcript.
    model_context_checkpoint: undefined,
  };
}
