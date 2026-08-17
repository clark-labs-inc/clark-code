import { describe, expect, it } from "vitest";
import {
  cleanVoiceContentType,
  createVoiceResamplerState,
  mergeVoiceTranscriptDraft,
  pcm16Bytes,
  resampleVoiceSamples,
  voiceElapsed,
  voiceLevel,
  voiceRecordingFilename,
} from "./voiceNarration";

describe("voice narration metadata", () => {
  it("replaces streaming hypotheses inside one dictated draft span", () => {
    const partial = mergeVoiceTranscriptDraft("Existing idea", null, "a focused spec");
    expect(partial.value).toBe("Existing idea a focused spec");

    const revised = mergeVoiceTranscriptDraft(
      partial.value,
      partial.session,
      "a focused spec for search",
    );
    expect(revised.value).toBe("Existing idea a focused spec for search");
  });

  it("normalizes recorder content types", () => {
    expect(cleanVoiceContentType("audio/webm;codecs=opus")).toBe("audio/webm");
    expect(cleanVoiceContentType()).toBe("audio/webm");
  });

  it("uses a semantic timestamped filename and readable elapsed time", () => {
    expect(voiceRecordingFilename("audio/mp4", new Date("2026-08-10T12:34:56.000Z")))
      .toBe("voice-narration-2026-08-10T12-34-56-000Z.m4a");
    expect(voiceElapsed(65_400)).toBe("1:05");
  });

  it("converts normalized microphone samples to little-endian PCM16", () => {
    const bytes = pcm16Bytes(new Float32Array([-1, 0, 1]));
    expect(Array.from(new Int16Array(bytes.buffer))).toEqual([-32768, 0, 32767]);
  });

  it("maps silence and speech energy to stable waveform levels", () => {
    expect(voiceLevel(new Float32Array(32))).toBe(0.06);
    expect(voiceLevel(new Float32Array(32).fill(0.01))).toBeCloseTo(5 / 12);
    expect(voiceLevel(new Float32Array(32).fill(0.1))).toBeCloseTo(5 / 6);
    expect(voiceLevel(new Float32Array(32).fill(1))).toBe(1);
  });

  it("resamples device-rate audio to the Clark 16 kHz streaming contract", () => {
    const state = createVoiceResamplerState();
    const first = resampleVoiceSamples(new Float32Array(480).fill(0.25), 48_000, state);
    const second = resampleVoiceSamples(new Float32Array(480).fill(0.25), 48_000, state);
    expect(first.length + second.length).toBe(320);
    expect(first.every((sample) => sample === 0.25)).toBe(true);
    expect(second.every((sample) => sample === 0.25)).toBe(true);
  });
});
