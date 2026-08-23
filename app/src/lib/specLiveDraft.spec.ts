import { describe, expect, it } from "vitest";

import { emptySnapshot, type Snapshot, type ToolCall, type ToolStatus } from "../core-bridge/types";
import {
  completedDocumentWrites,
  endsInsideCodeFence,
  specLiveDraft,
  splitStreamingMarkdown,
} from "./specLiveDraft";

function call(over: Partial<ToolCall> & { id: string }): ToolCall {
  return {
    title: "Writing a file",
    kind: "edit",
    status: "in_progress",
    locations: [],
    content: [],
    ...over,
  };
}

describe("specLiveDraft", () => {
  it("treats a streaming write as the document itself", () => {
    const draft = specLiveDraft([
      call({ id: "w", tool_name: "write_file", streamed_input: "# Spec\n\nStart " }),
    ]);

    expect(draft).toMatchObject({ kind: "document", text: "# Spec\n\nStart ", settled: false });
  });

  it("treats a streaming edit as a revision, not a replacement document", () => {
    // `new_string` is a fragment; presenting it as the spec would swap the whole
    // document for a snippet.
    const draft = specLiveDraft([
      call({ id: "e", tool_name: "edit_file", streamed_input: "## Safety boundaries" }),
    ]);

    expect(draft?.kind).toBe("revision");
  });

  it("marks the draft settled once arguments validate", () => {
    const draft = specLiveDraft([
      call({
        id: "w",
        tool_name: "write_file",
        streamed_input: "# Spec",
        raw_input: { path: "new_SPEC.md", content: "# Spec" },
      }),
    ]);

    expect(draft).toMatchObject({ settled: true, path: "new_SPEC.md" });
  });

  it("ignores tools whose payload is not a document", () => {
    expect(specLiveDraft([
      call({ id: "r", tool_name: "read_file", kind: "read", streamed_input: "irrelevant" }),
    ])).toBeNull();
    expect(specLiveDraft([
      call({ id: "t", tool_name: "computer_type_text", streamed_input: "hunter2" }),
    ])).toBeNull();
  });

  it("withholds a write that turns out to target a non-markdown file", () => {
    // The path is unknowable until arguments validate, so the gate tightens the
    // moment there is something to check.
    expect(specLiveDraft([
      call({
        id: "w",
        tool_name: "write_file",
        streamed_input: "fn main() {}",
        locations: [{ path: "src/main.rs" }],
      }),
    ])).toBeNull();
  });

  it.each<ToolStatus>(["failed", "cancelled"])(
    "drops the draft once the call reaches %s — its payload never landed",
    (status) => {
      expect(specLiveDraft([
        call({ id: "w", tool_name: "write_file", status, streamed_input: "# Spec" }),
      ])).toBeNull();
    },
  );

  it("keeps a completed document standing until the canonical read replaces it", () => {
    // The reducer releases streamed_input at completion; the validated
    // arguments carry the same text. Without this bridge the streamed document
    // vanishes into a "working" placeholder for the poll interval.
    const draft = specLiveDraft([
      call({
        id: "w",
        tool_name: "write_file",
        status: "completed",
        locations: [{ path: "new_SPEC.md" }],
        raw_input: { path: "new_SPEC.md", content: "# Spec\n\nBody.\n" },
      }),
    ]);

    expect(draft).toMatchObject({
      kind: "document",
      text: "# Spec\n\nBody.\n",
      settled: true,
    });
  });

  it("never bridges a completed write of a non-markdown file", () => {
    expect(specLiveDraft([
      call({
        id: "w",
        tool_name: "write_file",
        status: "completed",
        locations: [{ path: "src/main.rs" }],
        raw_input: { path: "src/main.rs", content: "fn main() {}" },
      }),
    ])).toBeNull();
  });

  it("lets a completed revision vanish — the settled diff owns that handoff", () => {
    expect(specLiveDraft([
      call({
        id: "e",
        tool_name: "edit_file",
        status: "completed",
        locations: [{ path: "new_SPEC.md" }],
        raw_input: { path: "new_SPEC.md", new_string: "## Changed" },
      }),
    ])).toBeNull();
  });

  it("follows the newest streaming write across a multi-step turn", () => {
    const draft = specLiveDraft([
      call({ id: "first", tool_name: "write_file", status: "completed", streamed_input: "# Old" }),
      call({ id: "second", tool_name: "write_file", streamed_input: "# New" }),
    ]);

    expect(draft?.callId).toBe("second");
  });
});

describe("splitStreamingMarkdown", () => {
  it("keeps the unfinished trailing line out of the settled prose", () => {
    expect(splitStreamingMarkdown("# Spec\n\n## Recomm")).toEqual({
      settled: "# Spec\n\n",
      live: "## Recomm",
    });
  });

  it("settles everything once the last line closes", () => {
    expect(splitStreamingMarkdown("# Spec\n")).toEqual({ settled: "# Spec\n", live: "" });
  });

  it("treats a first line with no break yet as entirely live", () => {
    expect(splitStreamingMarkdown("# Sp")).toEqual({ settled: "", live: "# Sp" });
    expect(splitStreamingMarkdown("")).toEqual({ settled: "", live: "" });
  });
});

function snapshotWith(calls: ToolCall[]): Snapshot {
  return {
    ...emptySnapshot(),
    tool_calls: Object.fromEntries(calls.map((c) => [c.id, c])),
  };
}

describe("completedDocumentWrites", () => {
  it("advances only when a document write has actually finished", () => {
    // The refresh trigger. Counting announcements instead would read the file
    // before it was written, which can only return the previous contents.
    const streaming = call({ id: "w", tool_name: "write_file", status: "in_progress" });
    expect(completedDocumentWrites(snapshotWith([streaming]))).toBe(0);
    expect(completedDocumentWrites(snapshotWith([{ ...streaming, status: "completed" }]))).toBe(1);
  });

  it("counts every tool that lands a document, and nothing else", () => {
    expect(completedDocumentWrites(snapshotWith([
      call({ id: "a", tool_name: "write_file", status: "completed" }),
      call({ id: "b", tool_name: "edit_file", status: "completed" }),
      call({ id: "c", tool_name: "apply_patch", status: "completed" }),
      call({ id: "d", tool_name: "read_file", status: "completed", kind: "read" }),
      call({ id: "e", tool_name: "bash", status: "completed", kind: "execute" }),
    ]))).toBe(3);
  });

  it("does not advance for a write that failed or was cancelled", () => {
    expect(completedDocumentWrites(snapshotWith([
      call({ id: "a", tool_name: "write_file", status: "failed" }),
      call({ id: "b", tool_name: "write_file", status: "cancelled" }),
    ]))).toBe(0);
  });

  it("is zero for a session that has written nothing", () => {
    expect(completedDocumentWrites(emptySnapshot())).toBe(0);
  });
});

describe("endsInsideCodeFence", () => {
  it("knows when the line being typed is inside an open fence", () => {
    expect(endsInsideCodeFence("Body.\n\n```bash\n")).toBe(true);
    expect(endsInsideCodeFence("Body.\n\n```bash\necho hi\n```\n")).toBe(false);
    expect(endsInsideCodeFence("- a list, not a fence\n")).toBe(false);
    expect(endsInsideCodeFence("")).toBe(false);
  });

  it("counts tilde fences and reopened fences the same way", () => {
    expect(endsInsideCodeFence("~~~\ncode\n~~~\n\n```diff\n")).toBe(true);
  });

  it("treats fence-lookalike lines inside an open fence as content, not closers", () => {
    // CommonMark: a closer must match the opener's character and be at least as
    // long, with no info string. A parity count would flip on every one of
    // these and style the line being typed as prose mid-snippet.
    expect(endsInsideCodeFence("~~~\n```\n")).toBe(true);
    expect(endsInsideCodeFence("````\n```\n")).toBe(true);
    expect(endsInsideCodeFence("```\n``` with trailing words\n")).toBe(true);
    expect(endsInsideCodeFence("~~~\n```\n~~~\n")).toBe(false);
    expect(endsInsideCodeFence("````\n```\n````\n")).toBe(false);
  });

  it("ignores a backtick run whose info string contains backticks", () => {
    // ``` ``` ``` on one line is paragraph text about fences, not a fence.
    expect(endsInsideCodeFence("``` `inline` ```\n")).toBe(false);
  });
});
