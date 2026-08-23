import type { ContentBlock, Snapshot, ToolCall } from "./types";

const RESERVED_PROTOCOL_MARKERS = [
  "begin_of_sentence",
  "require_escalated_model",
  "expiration_placeholder",
  "skillconstraint_hard",
] as const;

/** Any non-ASCII code point. NFKC is the identity on ASCII, so a value without
 *  one cannot gain a marker through normalization.
 *
 *  Deliberately a separate regex from the anchors below rather than one merged
 *  alternation: folding this character class into the alternation makes the
 *  whole pattern non-literal, which costs more than the second pass saves
 *  (measured 12.4 vs 9.9 us on a 9 KB diff). */
const NON_ASCII = /[^\x00-\x7F]/;

/** For each marker, its longest underscore-free run.
 *
 *  Normalization only ever *removes* underscore characters (a run of `[_▁]`
 *  collapses to one `_`) and case-folds. It never removes or reorders anything
 *  else, so two adjacent non-underscore characters in the normalized string
 *  were adjacent in the original. An underscore-free run of a marker therefore
 *  must appear literally, case-insensitively, in any ASCII string that could
 *  normalize to contain that marker. Failing this test proves the full test
 *  would fail, so it can only skip work — never change a verdict.
 *
 *  Keep in sync with RESERVED_PROTOCOL_MARKERS; the spec below pins that. */
const MARKER_ANCHORS = /sentence|escalated|placeholder|skillconstraint/i;

/** Test seam: proves each anchor really is an underscore-free run of a marker,
 *  which is the property the pre-filter's soundness rests on. */
export const RESERVED_MARKER_ANCHOR_SOURCE = MARKER_ANCHORS.source;
export const RESERVED_MARKERS_FOR_TEST = RESERVED_PROTOCOL_MARKERS;

function containsReservedProtocolMarker(value: string): boolean {
  // This runs over every agent message and every tool result on the arrival of
  // every host snapshot, so the common answer — "no marker here" — has to be
  // cheap. The normalizing form below allocates three copies of its input and
  // then scans each; on a transcript of code diffs that measured as the single
  // largest cost on the pre-render path. Two regex scans over the original
  // string answer the common case with no allocation at all.
  if (!NON_ASCII.test(value) && !MARKER_ANCHORS.test(value)) return false;
  const normalized = value.normalize("NFKC").toLowerCase().replace(/[_▁]+/gu, "_");
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
 *
 * Contamination is rare and this runs on the arrival of every host snapshot —
 * up to ~62 times a second per live session, before the render gate — so the
 * clean path is written to allocate nothing at all. Measured at a 320-turn
 * transcript, the previous `Object.entries` / `Set` / `timeline.filter` trio
 * cost about as much as the marker scanning it existed to support, and paid
 * that on every clean snapshot. Detection order and rejection semantics are
 * unchanged; only the allocation is deferred until a rejection is proven.
 */
export function quarantineSnapshotProviderOutput(snapshot: Snapshot): Snapshot {
  // Built lazily: null means "nothing rejected yet", which is the common case.
  let rejectedTools: Set<string> | null = null;
  for (const id of Object.keys(snapshot.tool_calls)) {
    if (toolCallIsContaminated(snapshot.tool_calls[id])) {
      (rejectedTools ??= new Set()).add(id);
    }
  }

  let contaminated = rejectedTools !== null || checkpointIsContaminated(snapshot);
  // Scan for a rejected timeline item without building a replacement array.
  // `contaminated` may already be true via the checkpoint, so this cannot stop
  // early on the first hit — every item still has to be classified below.
  let rejectedTimeline = false;
  for (const item of snapshot.timeline) {
    if (timelineItemIsRejected(item, rejectedTools)) {
      rejectedTimeline = true;
      break;
    }
  }
  contaminated ||= rejectedTimeline;
  if (!contaminated) return snapshot;

  // Rejection path: now the allocations are earned.
  const timeline = rejectedTimeline
    ? snapshot.timeline.filter((item) => !timelineItemIsRejected(item, rejectedTools))
    : snapshot.timeline;
  const toolCalls = rejectedTools === null
    ? snapshot.tool_calls
    : Object.fromEntries(
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

/** One timeline item's verdict. Shared by the detection scan and the filter so
 *  the two can never disagree about what gets removed. */
function timelineItemIsRejected(
  item: Snapshot["timeline"][number],
  rejectedTools: Set<string> | null,
): boolean {
  if (item.item === "message") {
    return item.role === "agent" && blocksAreContaminated(item.blocks);
  }
  return item.item === "tool_call" && rejectedTools !== null && rejectedTools.has(item.id);
}
