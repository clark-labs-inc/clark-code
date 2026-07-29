import { describe, expect, it } from "vitest";
import { nextTextRevealPosition, synchronizedRevealPosition } from "./useSmoothText";

describe("streamed text reveal", () => {
  it("uses a small visible step for ordinary text and stops at the end", () => {
    expect(nextTextRevealPosition("abcdef", 0)).toBe(2);
    expect(nextTextRevealPosition("abcdef", 6)).toBe(6);
  });

  it("drains a large burst proportionally instead of one character at a time", () => {
    expect(nextTextRevealPosition("a".repeat(120), 0)).toBe(10);
  });

  it("never splits emoji ZWJ sequences or combining-mark graphemes", () => {
    const text = "A👩‍💻e\u0301Z";
    const afterEmoji = nextTextRevealPosition(text, 0);
    const afterAccent = nextTextRevealPosition(text, afterEmoji);

    expect(text.slice(0, afterEmoji)).toBe("A👩‍💻");
    expect(text.slice(0, afterAccent)).toBe("A👩‍💻e\u0301");
  });

  it("does not replay a settled message when a stream starts again", () => {
    expect(synchronizedRevealPosition(7, 11, true, false)).toBe(11);
    expect(synchronizedRevealPosition(0, 11, true, true)).toBe(0);
  });
});
