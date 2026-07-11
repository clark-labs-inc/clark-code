import { describe, expect, it } from "vitest";
import { parseNarration } from "./narration";

describe("parseNarration", () => {
  it("returns a single text span when there are no tags", () => {
    expect(parseNarration("Hello world")).toEqual([{ kind: "text", text: "Hello world" }]);
  });

  it("splits narrate and thinking from surrounding text", () => {
    const spans = parseNarration(
      "Before <thinking>let me reason</thinking> mid <narrate>I'll search now</narrate> after",
    );
    expect(spans).toEqual([
      { kind: "text", text: "Before" },
      { kind: "thinking", text: "let me reason" },
      { kind: "text", text: "mid" },
      { kind: "narrate", text: "I'll search now" },
      { kind: "text", text: "after" },
    ]);
  });

  it("treats an unclosed tag (mid-stream) as that kind to the end", () => {
    const spans = parseNarration("Done. <thinking>still thinking");
    expect(spans).toEqual([
      { kind: "text", text: "Done." },
      { kind: "thinking", text: "still thinking" },
    ]);
  });

  it("accepts <narration> as an alias and coalesces adjacent same-kind", () => {
    const spans = parseNarration("<narration>a</narration><narrate>b</narrate>");
    expect(spans).toEqual([{ kind: "narrate", text: "ab" }]);
  });

  it("renders a native reasoning block as a thinking span", () => {
    // A GLM `delta.reasoning` content block is flattened (Message.text) to an
    // inline <thinking> tag; parseNarration must split it into a thinking span
    // so it renders as the collapsible Thinking row, not as plain answer text.
    const spans = parseNarration("Sure. <thinking>weighing options…</thinking> Done.");
    expect(spans).toEqual([
      { kind: "text", text: "Sure." },
      { kind: "thinking", text: "weighing options…" },
      { kind: "text", text: "Done." },
    ]);
  });
});
