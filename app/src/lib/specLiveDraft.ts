import type { Snapshot, ToolCall } from "../core-bridge/types";

/** What a streamed payload actually is. `write_file.content` is the whole
 *  document, so it can stand in for the spec while it is being typed.
 *  `edit_file.new_string` is a fragment — showing it as "the document" would
 *  replace the spec with a snippet, so it surfaces as an incoming revision. */
type SpecDraftKind = "document" | "revision";

export interface SpecLiveDraft {
  kind: SpecDraftKind;
  /** Decoded markdown received so far. */
  text: string;
  /** Arguments have validated, so `text` is final for this call — but the file
   *  is not written yet, so the canonical document is still behind this. */
  settled: boolean;
  path?: string;
  callId: string;
}

/** Mirrors the provider's streaming allowlist (`document_stream.rs`): only these
 *  two tools ever carry `streamed_input`, and only for markdown targets. */
const DRAFT_KIND: Record<string, SpecDraftKind> = {
  write_file: "document",
  edit_file: "revision",
};

/** Tools whose completion means a document file may have changed on disk. Wider
 *  than DRAFT_KIND: `apply_patch` never streams (its payload is a code patch
 *  with no single target to gate on), but it can still modify the spec file, so
 *  its completion must still trigger a re-read. */
const REFRESH_TOOLS = new Set(["write_file", "edit_file", "apply_patch"]);

/** Markdown is the only thing the Spec surface can present. A run may touch
 *  other files; those must not masquerade as the spec. */
function isMarkdownPath(path: string): boolean {
  return /\.(?:md|markdown)$/i.test(path);
}

function draftPath(call: ToolCall): string | undefined {
  const fromLocation = call.locations?.[0]?.path;
  if (fromLocation) return fromLocation;
  const input = call.raw_input as { path?: unknown } | undefined;
  return typeof input?.path === "string" ? input.path : undefined;
}

/** The document the model is currently typing, if any.
 *
 *  Ordering note: the path is unknowable until arguments validate, because the
 *  provider announces a tool call before its arguments parse. So the gate widens
 *  from "a write is streaming" to "a write is streaming *to a markdown file*" the
 *  moment there is a path to check — never showing a non-markdown payload as the
 *  spec, and never withholding the first draft while it is most useful. */
export function specLiveDraft(calls: readonly ToolCall[]): SpecLiveDraft | null {
  for (let index = calls.length - 1; index >= 0; index -= 1) {
    const call = calls[index];
    const kind = call.tool_name ? DRAFT_KIND[call.tool_name] : undefined;
    if (!kind) continue;
    // A completed full-document write keeps standing in for the spec until the
    // canonical file read replaces it. The reducer releases `streamed_input` at
    // terminal status, but the validated arguments carry the same text — and
    // without this bridge the streamed document would vanish into a "working"
    // placeholder for the up-to-350 ms gap before the poll reads the file.
    if (call.status === "completed") {
      if (kind !== "document") continue;
      const path = draftPath(call);
      if (!path || !isMarkdownPath(path)) continue;
      const payload = (call.raw_input as { content?: unknown } | undefined)?.content;
      if (typeof payload !== "string" || !payload.trim()) continue;
      return { kind, text: payload, settled: true, path, callId: call.id };
    }
    // A failed or cancelled write landed nothing; its payload must not present
    // itself as the document.
    if (call.status === "failed" || call.status === "cancelled") continue;
    const text = call.streamed_input ?? "";
    if (!text) continue;
    const path = draftPath(call);
    if (path && !isMarkdownPath(path)) continue;
    return {
      kind,
      text,
      settled: call.raw_input != null,
      path,
      callId: call.id,
    };
  }
  return null;
}

/** Whether `settled` markdown ends inside an open ``` / ~~~ fence, so the line
 *  being typed is code. A leading `- ` there is diff or shell syntax, not a
 *  list marker, and the live line should read as code, not prose.
 *
 *  This is a scanner, not a parity count, because fence lines are not toggles:
 *  a ``` line inside an open ~~~ fence is content, a ``` line inside a ````
 *  fence is content (closers must be at least as long as their opener), and a
 *  closer may not carry an info string. A parity count gets every one of those
 *  wrong and flips the style of the line being typed. */
export function endsInsideCodeFence(settled: string): boolean {
  let open: { char: string; length: number } | null = null;
  for (const line of settled.split("\n")) {
    const fence = /^ {0,3}(`{3,}|~{3,})(.*)$/.exec(line);
    if (!fence) continue;
    const marker = fence[1];
    const char = marker[0];
    if (!open) {
      // A backtick opener's info string may not contain backticks; such a line
      // is paragraph text, not a fence.
      if (char === "`" && fence[2].includes("`")) continue;
      open = { char, length: marker.length };
    } else if (char === open.char && marker.length >= open.length && fence[2].trim() === "") {
      open = null;
    }
  }
  return open !== null;
}

/** Split a streaming markdown document into the part that is safe to render as
 *  settled prose and the trailing line still being typed. Keeping the live line
 *  separate is what lets Pretext hold its final geometry while words land in it,
 *  instead of the whole document reflowing on every token. */
export function splitStreamingMarkdown(text: string): { settled: string; live: string } {
  if (!text) return { settled: "", live: "" };
  // A trailing newline means even the last line is complete.
  const lastBreak = text.lastIndexOf("\n");
  if (lastBreak === text.length - 1) return { settled: text, live: "" };
  if (lastBreak < 0) return { settled: "", live: text };
  return { settled: text.slice(0, lastBreak + 1), live: text.slice(lastBreak + 1) };
}

/** Whether this call's completion means a document file changed on disk. */
function isDocumentWrite(call: ToolCall): boolean {
  return Boolean(call.tool_name && REFRESH_TOOLS.has(call.tool_name));
}

/** A monotonic count of finished document writes.
 *
 *  This advances at exactly the moment a file lands on disk, which is the only
 *  moment a re-read can return something new. Using it as a refresh trigger is
 *  what lets the document reload promptly *without* reading on every timeline
 *  item — most of which are announcements of work that has not written anything
 *  yet, so reading then can only return the previous contents. */
export function completedDocumentWrites(snapshot: Snapshot): number {
  let count = 0;
  for (const call of Object.values(snapshot.tool_calls)) {
    if (call.status === "completed" && isDocumentWrite(call)) count += 1;
  }
  return count;
}
