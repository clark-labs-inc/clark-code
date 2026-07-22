import { describe, expect, it } from "vitest";
import {
  createPendingPaste,
  expandPendingPastes,
  fileToAttachment,
  prettySize,
  shouldThumbnailPastedText,
  toUpload,
} from "./attachments";

describe("fileToAttachment", () => {
  it("encodes a non-image file to a base64 upload", async () => {
    const file = new File(["hello world"], "note.txt", { type: "text/plain" });
    const att = await fileToAttachment(file);
    expect(att.filename).toBe("note.txt");
    expect(att.content_type).toBe("text/plain");
    expect(att.size).toBe(11);
    expect(atob(att.data_base64)).toBe("hello world");
  });

  it("toUpload keeps only the wire fields", async () => {
    const file = new File(["x"], "a.bin", { type: "application/octet-stream" });
    const att = await fileToAttachment(file);
    expect(toUpload(att)).toEqual({
      filename: "a.bin",
      content_type: "application/octet-stream",
      data_base64: att.data_base64,
    });
  });

  it("defaults a missing content type", async () => {
    const file = new File(["x"], "mystery");
    const att = await fileToAttachment(file);
    expect(att.content_type).toBe("application/octet-stream");
  });
});

describe("prettySize", () => {
  it("formats bytes / KB / MB", () => {
    expect(prettySize(500)).toBe("500 B");
    expect(prettySize(2048)).toBe("2 KB");
    expect(prettySize(3 * 1024 * 1024)).toBe("3.0 MB");
  });
});

describe("large pasted text", () => {
  it("compacts only text over the 1,000-character boundary", () => {
    expect(shouldThumbnailPastedText("short paste")).toBe(false);
    expect(shouldThumbnailPastedText("x".repeat(1_000))).toBe(false);
    expect(shouldThumbnailPastedText("x".repeat(1_001))).toBe(true);
    expect(shouldThumbnailPastedText("😀".repeat(1_001))).toBe(true);
  });

  it("creates collision-free placeholders and expands them in place", () => {
    const text = "x".repeat(1_001);
    const first = createPendingPaste(text, []);
    const second = createPendingPaste(text, [first]);
    expect(first.placeholder).toBe("[Pasted Content 1001 chars]");
    expect(second.placeholder).toBe("[Pasted Content 1001 chars] #2");
    expect(
      expandPendingPastes(`before ${first.placeholder} middle ${second.placeholder} after`, [
        first,
        second,
      ]),
    ).toBe(`before ${text} middle ${text} after`);
    expect(expandPendingPastes("typed request", [first])).toBe(`typed request\n\n${text}`);
  });
});
