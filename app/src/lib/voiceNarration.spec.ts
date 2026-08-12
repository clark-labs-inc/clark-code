import { describe, expect, it } from "vitest";
import {
  cleanVoiceContentType,
  voiceElapsed,
  voiceRecordingFilename,
} from "./voiceNarration";

describe("voice narration metadata", () => {
  it("normalizes recorder content types", () => {
    expect(cleanVoiceContentType("audio/webm;codecs=opus")).toBe("audio/webm");
    expect(cleanVoiceContentType()).toBe("audio/webm");
  });

  it("uses a semantic timestamped filename and readable elapsed time", () => {
    expect(voiceRecordingFilename("audio/mp4", new Date("2026-08-10T12:34:56.000Z")))
      .toBe("voice-narration-2026-08-10T12-34-56-000Z.m4a");
    expect(voiceElapsed(65_400)).toBe("1:05");
  });
});
