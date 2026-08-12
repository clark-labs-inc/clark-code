export type VoiceRecordingPhase = "idle" | "recording" | "transcribing";

const MIME_TYPES = [
  "audio/webm;codecs=opus",
  "audio/webm",
  "audio/mp4",
  "audio/ogg;codecs=opus",
  "audio/ogg",
] as const;

export function supportsVoiceRecording(): boolean {
  return typeof window !== "undefined"
    && typeof window.MediaRecorder !== "undefined"
    && typeof navigator !== "undefined"
    && Boolean(navigator.mediaDevices?.getUserMedia);
}

export function preferredVoiceMimeType(): string | undefined {
  if (!supportsVoiceRecording()) return undefined;
  return MIME_TYPES.find((candidate) => window.MediaRecorder.isTypeSupported(candidate));
}

export function cleanVoiceContentType(value?: string): string {
  return value?.split(";")[0]?.trim() || "audio/webm";
}

export function voiceRecordingFilename(contentType: string, now = new Date()): string {
  const extension = (() => {
    switch (cleanVoiceContentType(contentType).toLowerCase()) {
      case "audio/mp4":
      case "audio/m4a":
      case "audio/x-m4a":
        return "m4a";
      case "audio/ogg":
      case "audio/oga":
        return "ogg";
      case "audio/wav":
      case "audio/wave":
      case "audio/x-wav":
        return "wav";
      default:
        return "webm";
    }
  })();
  return `voice-narration-${now.toISOString().replace(/[:.]/g, "-")}.${extension}`;
}

export function voiceElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

export async function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error("Could not read voice recording"));
    reader.onload = () => {
      if (typeof reader.result !== "string") {
        reject(new Error("Could not encode voice recording"));
        return;
      }
      const separator = reader.result.indexOf(",");
      if (separator < 0) {
        reject(new Error("Voice recording encoding was malformed"));
        return;
      }
      resolve(reader.result.slice(separator + 1));
    };
    reader.readAsDataURL(blob);
  });
}

export function voiceCaptureMessage(error: unknown): string {
  const name = error instanceof DOMException ? error.name : "";
  switch (name) {
    case "NotAllowedError":
    case "SecurityError":
      return "Microphone access is needed for narration.";
    case "NotFoundError":
    case "OverconstrainedError":
      return "No microphone is available.";
    case "NotReadableError":
      return "The microphone is in use by another application.";
    default:
      return error instanceof Error ? error.message : "Voice recording could not start.";
  }
}
