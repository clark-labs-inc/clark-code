import { describe, expect, it } from "vitest";
import {
  attachmentKind,
  createPendingPaste,
  expandPendingPastes,
  fileToAttachment,
  prettySize,
  restorePendingAttachments,
  shouldThumbnailPastedText,
  toUpload,
} from "./attachments";

describe("attachmentKind", () => {
  it("classifies model-facing attachment families from MIME and extension", () => {
    expect(attachmentKind("note.txt", "text/plain")).toBe("text");
    expect(attachmentKind("scan.PDF", "application/octet-stream")).toBe("pdf");
    expect(attachmentKind("report.docx", "application/zip")).toBe("docx");
    expect(attachmentKind("voice.wav", "audio/wav")).toBe("audio");
    expect(attachmentKind("archive.bin", "application/octet-stream")).toBe("binary");
  });
});

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

  it("preserves accepted image bytes, MIME type, size, and filename", async () => {
    const bytes = new Uint8Array(1_300_000);
    bytes[0] = 0x89;
    bytes[1] = 0x50;
    bytes[bytes.length - 1] = 0x7f;
    const file = new File([bytes], "full-resolution.png", { type: "image/png" });

    const att = await fileToAttachment(file);

    expect(att.filename).toBe("full-resolution.png");
    expect(att.content_type).toBe("image/png");
    expect(att.size).toBe(bytes.length);
    const roundTrip = Uint8Array.from(atob(att.data_base64), (char) => char.charCodeAt(0));
    expect(roundTrip).toEqual(bytes);
  }, 15_000);
});

describe("prettySize", () => {
  it("formats bytes / KB / MB", () => {
    expect(prettySize(500)).toBe("500 B");
    expect(prettySize(2048)).toBe("2 KB");
    expect(prettySize(3 * 1024 * 1024)).toBe("3.0 MB");
  });
});

describe("restorePendingAttachments", () => {
  it("restores a rejected payload ahead of newly staged files without duplicates", () => {
    const submitted = {
      id: "submitted",
      filename: "submitted.png",
      content_type: "image/png",
      data_base64: "cG5n",
      size: 3,
    };
    const current = {
      id: "current",
      filename: "current.txt",
      content_type: "text/plain",
      data_base64: "dHh0",
      size: 3,
    };

    expect(restorePendingAttachments([submitted], [current])).toEqual([submitted, current]);
    expect(restorePendingAttachments([submitted], [submitted, current])).toEqual([
      submitted,
      current,
    ]);
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
