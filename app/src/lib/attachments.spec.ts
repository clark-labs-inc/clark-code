import { describe, expect, it } from "vitest";
import { fileToAttachment, toUpload, prettySize } from "./attachments";

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
