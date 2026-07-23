// Bound the size of a conversation snapshot before it syncs to the cloud.
//
// The cloud blob is the source of truth for chats (no local persistence), and
// it accretes full tool outputs verbatim over a conversation's life — a single
// file read, command dump, web fetch, or base64 screenshot can be megabytes.
// Left unbounded, a long conversation eventually exceeds the sync ceiling and
// stops syncing silently. This module trims a COPY for upload: only when a
// snapshot is over target, and only the OLDEST tool calls, whose bulky outputs
// are elided to short previews while the newest turns stay verbatim. Mirrors
// the agent loop's own "truncate oldest tool result content" context transform.
//
// The live in-memory snapshot the desktop renders is never mutated — only the
// uploaded copy shrinks, so reopening a very long conversation on another
// device shows recent turns in full and older bulky outputs as elided markers.
// For normal-sized conversations the snapshot is passed through untouched.

import type { ContentBlock, Snapshot, ToolCall } from "../core-bridge/types";

/** Below this serialized size a snapshot uploads untouched — the common case.
 *  Sits well under the server's body ceiling so the envelope + meta fields
 *  never push an accepted snapshot over the limit. */
export const SYNC_TARGET_BYTES = 1_500_000;

/** Newest tool calls kept fully intact; only older ones get elided. */
export const KEEP_RECENT_TOOL_CALLS = 12;

/** Text longer than this in an old tool call is cut to a leading preview. */
export const TEXT_PREVIEW_CHARS = 400;

export interface PreparedSnapshot {
  /** The snapshot to upload (the original when under target, else a trimmed
   *  clone). */
  snapshot: Snapshot;
  /** Its serialized form — reused by the caller for the fingerprint and the
   *  hard-cap check so nothing stringifies the blob twice. */
  json: string;
  /** UTF-8 byte length of `json`, matching HTTP body accounting. */
  bytes: number;
  /** True when old tool outputs were elided to fit. */
  elided: boolean;
}

/** Serialized request bodies are UTF-8; JavaScript string length counts UTF-16
 * code units and can undercount non-ASCII history by almost 2×. */
export function utf8ByteLength(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

/** Human-readable elided-byte size for the placeholder markers. */
function humanSize(chars: number): string {
  return chars >= 1024 ? `${Math.round(chars / 1024)} KB` : `${chars} B`;
}

/** Elide one content block, returning the replacement and an estimate of the
 *  characters removed (used only to decide when enough has been trimmed). */
function elideBlock(block: ContentBlock): [ContentBlock, number] {
  if (block.type === "text") {
    if (block.text.length <= TEXT_PREVIEW_CHARS) return [block, 0];
    const kept = block.text.slice(0, TEXT_PREVIEW_CHARS);
    const removed = block.text.length - kept.length;
    return [
      { type: "text", text: `${kept}\n… (${humanSize(removed)} elided from history)` },
      removed,
    ];
  }
  if (block.type === "image" || block.type === "audio") {
    const removed = block.data.length;
    if (removed === 0) return [block, 0];
    return [
      { type: "text", text: `[${block.type} elided from history — ${humanSize(removed)}]` },
      removed,
    ];
  }
  if (block.type === "resource" && block.text && block.text.length > TEXT_PREVIEW_CHARS) {
    const kept = block.text.slice(0, TEXT_PREVIEW_CHARS);
    const removed = block.text.length - kept.length;
    return [{ ...block, text: `${kept}\n… (${humanSize(removed)} elided)` }, removed];
  }
  return [block, 0];
}

/** Elide a tool call's bulky output in place, returning ~chars removed. */
function elideToolCall(call: ToolCall): number {
  let saved = 0;
  if (call.raw_input !== undefined) {
    saved += JSON.stringify(call.raw_input)?.length ?? 0;
    delete call.raw_input;
  }
  call.content = call.content.map((block) => {
    const [next, removed] = elideBlock(block);
    saved += removed;
    return next;
  });
  return saved;
}

/**
 * Prepare a snapshot for cloud upload, trimming old tool outputs only if the
 * serialized snapshot exceeds `targetBytes`. Pure: never mutates `snapshot`.
 */
export function prepareSnapshotForUpload(
  snapshot: Snapshot,
  targetBytes: number = SYNC_TARGET_BYTES,
): PreparedSnapshot {
  const json = JSON.stringify(snapshot);
  const bytes = utf8ByteLength(json);
  if (bytes <= targetBytes) return { snapshot, json, bytes, elided: false };

  // Clone via the string we already built — one parse, no second stringify of
  // the original — then trim the clone so the live snapshot is untouched.
  const clone = JSON.parse(json) as Snapshot;
  const toolIds = clone.timeline
    .filter((item): item is { item: "tool_call"; id: string } => item.item === "tool_call")
    .map((item) => item.id);
  // Oldest first, sparing the most recent KEEP_RECENT_TOOL_CALLS.
  const trimable = toolIds.slice(0, Math.max(0, toolIds.length - KEEP_RECENT_TOOL_CALLS));

  // Estimate is approximate (JSON escaping, added markers); the caller's
  // hard-cap check on the returned `json` is the real gate. Erring toward
  // under-trim only risks a slightly larger — never rejected — payload.
  let estimate = bytes;
  let elided = false;
  for (const id of trimable) {
    if (estimate <= targetBytes) break;
    const call = clone.tool_calls[id];
    if (!call) continue;
    const removed = elideToolCall(call);
    if (removed > 0) {
      estimate -= removed;
      elided = true;
    }
  }

  const preparedJson = JSON.stringify(clone);
  return {
    snapshot: clone,
    json: preparedJson,
    bytes: utf8ByteLength(preparedJson),
    elided,
  };
}
