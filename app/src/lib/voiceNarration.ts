export type VoiceRecordingPhase = "idle" | "connecting" | "recording" | "transcribing";

export interface VoiceDraftSession {
  prefix: string;
}

export function mergeVoiceTranscriptDraft(
  current: string,
  session: VoiceDraftSession | null,
  transcript: string,
): { value: string; session: VoiceDraftSession } {
  const text = transcript.trim();
  const nextSession = session ?? (() => {
    const existing = current.trimEnd();
    return { prefix: `${existing}${existing ? " " : ""}` };
  })();
  return {
    value: `${nextSession.prefix}${text}`,
    session: nextSession,
  };
}

const MIME_TYPES = [
  "audio/webm;codecs=opus",
  "audio/webm",
  "audio/mp4",
  "audio/ogg;codecs=opus",
  "audio/ogg",
] as const;

export function supportsVoiceRecording(streaming = false): boolean {
  return typeof window !== "undefined"
    && typeof navigator !== "undefined"
    && Boolean(navigator.mediaDevices?.getUserMedia)
    && (streaming
      ? typeof window.AudioContext !== "undefined" && typeof window.AudioWorkletNode !== "undefined"
      : typeof window.MediaRecorder !== "undefined");
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

export function pcm16Bytes(samples: Float32Array): Uint8Array {
  const pcm = new Int16Array(samples.length);
  for (let index = 0; index < samples.length; index += 1) {
    const sample = Math.max(-1, Math.min(1, samples[index]));
    pcm[index] = sample < 0 ? sample * 32768 : sample * 32767;
  }
  return new Uint8Array(pcm.buffer);
}

export function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunkSize = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + chunkSize));
  }
  return window.btoa(binary);
}

export function voiceLevel(samples: Float32Array): number {
  if (samples.length === 0) return 0;
  let sumSquares = 0;
  for (const sample of samples) sumSquares += sample * sample;
  const rms = Math.sqrt(sumSquares / samples.length);
  if (rms <= 0.001) return 0.06;
  const decibels = 20 * Math.log10(rms);
  return Math.max(0.06, Math.min(1, (decibels + 60) / 48));
}

export interface VoiceResamplerState {
  position: number;
  remainder: Float32Array;
}

export function createVoiceResamplerState(): VoiceResamplerState {
  return { position: 0, remainder: new Float32Array(0) };
}

export function resampleVoiceSamples(
  samples: Float32Array,
  inputRate: number,
  state: VoiceResamplerState,
): Float32Array {
  if (!Number.isFinite(inputRate) || inputRate <= 0) return new Float32Array(0);
  const combined = new Float32Array(state.remainder.length + samples.length);
  combined.set(state.remainder);
  combined.set(samples, state.remainder.length);
  const ratio = inputRate / 16_000;
  const output: number[] = [];
  while (state.position + 1 < combined.length) {
    const left = Math.floor(state.position);
    const fraction = state.position - left;
    output.push(combined[left] * (1 - fraction) + combined[left + 1] * fraction);
    state.position += ratio;
  }
  const consumed = Math.min(Math.floor(state.position), combined.length);
  state.remainder = combined.slice(consumed);
  state.position -= consumed;
  return Float32Array.from(output);
}

export interface VoicePcmCapture {
  close: () => Promise<void>;
}

const PCM_WORKLET = `
class ClarkVoicePcmProcessor extends AudioWorkletProcessor {
  process(inputs) {
    const samples = inputs[0] && inputs[0][0];
    if (samples && samples.length) {
      const copy = samples.slice();
      this.port.postMessage(copy.buffer, [copy.buffer]);
    }
    return true;
  }
}
registerProcessor("clark-voice-pcm", ClarkVoicePcmProcessor);
`;

export async function startVoicePcmCapture(
  stream: MediaStream,
  onAudio: (dataBase64: string) => void,
  onLevel: (level: number) => void,
): Promise<VoicePcmCapture> {
  const samplesPerFrame = 1_600;
  const context = new AudioContext({ sampleRate: 16_000 });
  const moduleUrl = URL.createObjectURL(new Blob([PCM_WORKLET], { type: "text/javascript" }));
  try {
    await context.audioWorklet.addModule(moduleUrl);
  } finally {
    URL.revokeObjectURL(moduleUrl);
  }
  const source = context.createMediaStreamSource(stream);
  const worklet = new AudioWorkletNode(context, "clark-voice-pcm");
  const silentOutput = context.createGain();
  let pending = new Int16Array(0);
  let pendingLevel = 0;
  const resampler = createVoiceResamplerState();
  silentOutput.gain.value = 0;
  worklet.port.onmessage = (event: MessageEvent<ArrayBuffer>) => {
    const microphoneSamples = new Float32Array(event.data);
    const samples = resampleVoiceSamples(microphoneSamples, context.sampleRate, resampler);
    if (samples.length === 0) return;
    pendingLevel = Math.max(pendingLevel, voiceLevel(samples));
    const incoming = new Int16Array(pcm16Bytes(samples).buffer);
    const combined = new Int16Array(pending.length + incoming.length);
    combined.set(pending);
    combined.set(incoming, pending.length);
    let offset = 0;
    while (combined.length - offset >= samplesPerFrame) {
      onAudio(bytesToBase64(new Uint8Array(
        combined.buffer,
        combined.byteOffset + offset * Int16Array.BYTES_PER_ELEMENT,
        samplesPerFrame * Int16Array.BYTES_PER_ELEMENT,
      )));
      onLevel(pendingLevel);
      pendingLevel = 0;
      offset += samplesPerFrame;
    }
    pending = combined.slice(offset);
  };
  source.connect(worklet);
  worklet.connect(silentOutput);
  silentOutput.connect(context.destination);
  await context.resume();
  return {
    close: async () => {
      worklet.port.onmessage = null;
      if (pending.length > 0) {
        onAudio(bytesToBase64(new Uint8Array(
          pending.buffer,
          pending.byteOffset,
          pending.byteLength,
        )));
        onLevel(pendingLevel);
        pending = new Int16Array(0);
      }
      source.disconnect();
      worklet.disconnect();
      silentOutput.disconnect();
      await context.close();
    },
  };
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
