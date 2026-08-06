import { describe, expect, it } from "vitest";
import { privacySafeDiagnosticReference, privacySafeStackFrames } from "./localCapture";

describe("local capture privacy boundary", () => {
  it("turns raw exception content into an opaque stable reference", () => {
    const reference = privacySafeDiagnosticReference(
      "Error",
      "token=secret conversation=private",
      "Error: token=secret\n at Conversation.tsx:42",
    );

    expect(reference).toMatch(/^DESKTOP-[0-9A-F]{8}$/);
    expect(reference).not.toContain("secret");
    expect(reference).not.toContain("private");
  });

  it("removes the message-bearing first stack line", () => {
    const error = new Error("token=secret conversation=private");
    error.stack = "Error: token=secret conversation=private\n    at Conversation.tsx:42:7";

    const frames = privacySafeStackFrames(error);

    expect(frames).toContain("Conversation.tsx:42:7");
    expect(frames).not.toContain("token=secret");
    expect(frames).not.toContain("conversation=private");
  });
});
