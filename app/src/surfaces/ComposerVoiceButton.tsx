import { useCallback, useEffect, useRef, useState } from "react";
import { Loader2, Mic, Square } from "lucide-react";
import { productModule } from "../product/productModule";
import {
  blobToBase64,
  cleanVoiceContentType,
  preferredVoiceMimeType,
  supportsVoiceRecording,
  voiceCaptureMessage,
  voiceElapsed,
  voiceRecordingFilename,
  type VoiceRecordingPhase,
} from "../lib/voiceNarration";
import { cn } from "../lib/cn";

export function ComposerVoiceButton({
  disabled,
  onTranscript,
  onError,
}: {
  disabled?: boolean;
  onTranscript: (text: string) => void;
  onError: (message: string) => void;
}) {
  const transcriber = productModule().voice?.transcribe;
  const [phase, setPhase] = useState<VoiceRecordingPhase>("idle");
  const [elapsed, setElapsed] = useState(0);
  const recorderRef = useRef<MediaRecorder | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const startedAtRef = useRef(0);

  const stopStream = useCallback(() => {
    streamRef.current?.getTracks().forEach((track) => track.stop());
    streamRef.current = null;
  }, []);

  const stopToBlob = useCallback(() => new Promise<Blob>((resolve, reject) => {
    const recorder = recorderRef.current;
    if (!recorder) {
      reject(new Error("No active voice recording"));
      return;
    }
    const cleanup = () => {
      recorder.removeEventListener("stop", finish);
      recorder.removeEventListener("error", fail);
    };
    const finish = () => {
      cleanup();
      resolve(new Blob(chunksRef.current, {
        type: recorder.mimeType || chunksRef.current[0]?.type || "audio/webm",
      }));
    };
    const fail = () => {
      cleanup();
      reject(new Error("Voice recording failed"));
    };
    recorder.addEventListener("stop", finish);
    recorder.addEventListener("error", fail);
    try {
      recorder.requestData();
      recorder.stop();
    } catch (error) {
      cleanup();
      reject(error);
    }
  }), []);

  const start = useCallback(async () => {
    if (disabled || phase !== "idle") return;
    if (!transcriber) {
      onError("Voice narration is unavailable in this Clark Code build.");
      return;
    }
    if (!supportsVoiceRecording()) {
      onError("Voice recording is unavailable on this device.");
      return;
    }
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const mimeType = preferredVoiceMimeType();
      const recorder = new MediaRecorder(stream, mimeType ? { mimeType } : undefined);
      chunksRef.current = [];
      recorder.addEventListener("dataavailable", (event) => {
        if (event.data.size > 0) chunksRef.current.push(event.data);
      });
      recorder.addEventListener("error", () => {
        stopStream();
        setPhase("idle");
        onError("Voice recording failed.");
      });
      streamRef.current = stream;
      recorderRef.current = recorder;
      startedAtRef.current = Date.now();
      setElapsed(0);
      setPhase("recording");
      recorder.start(1_000);
    } catch (error) {
      stopStream();
      recorderRef.current = null;
      setPhase("idle");
      onError(voiceCaptureMessage(error));
    }
  }, [disabled, onError, phase, stopStream, transcriber]);

  const finish = useCallback(async () => {
    if (phase !== "recording" || !transcriber) return;
    setPhase("transcribing");
    try {
      const blob = await stopToBlob();
      stopStream();
      recorderRef.current = null;
      chunksRef.current = [];
      if (blob.size === 0) throw new Error("No speech was recorded.");
      const contentType = cleanVoiceContentType(blob.type);
      const result = await transcriber({
        filename: voiceRecordingFilename(contentType),
        contentType,
        dataBase64: await blobToBase64(blob),
      });
      if (!result.text.trim()) throw new Error("No speech was detected.");
      onTranscript(result.text.trim());
    } catch (error) {
      onError(error instanceof Error ? error.message : "Voice transcription failed.");
    } finally {
      stopStream();
      recorderRef.current = null;
      chunksRef.current = [];
      setElapsed(0);
      setPhase("idle");
    }
  }, [onError, onTranscript, phase, stopStream, stopToBlob, transcriber]);

  useEffect(() => {
    if (phase !== "recording") return;
    const timer = window.setInterval(() => setElapsed(Date.now() - startedAtRef.current), 250);
    return () => window.clearInterval(timer);
  }, [phase]);

  useEffect(() => () => {
    if (recorderRef.current?.state === "recording") recorderRef.current.stop();
    stopStream();
  }, [stopStream]);

  if (!transcriber) return null;

  const recording = phase === "recording";
  const transcribing = phase === "transcribing";
  return (
    <button
      type="button"
      onClick={() => recording ? void finish() : void start()}
      disabled={disabled || transcribing}
      aria-label={recording ? "Stop voice narration and transcribe" : "Start voice narration"}
      title={recording ? `Stop and transcribe · ${voiceElapsed(elapsed)}` : "Narrate your idea"}
      className={cn(
        "flex h-8 shrink-0 items-center gap-1.5 rounded-full px-2 text-xs font-medium transition",
        recording
          ? "bg-danger/12 text-danger"
          : "text-ink-muted hover:bg-accent-subtle hover:text-accent",
        transcribing && "text-accent",
      )}
    >
      {transcribing ? (
        <Loader2 className="size-4 animate-[spin_1s_linear_infinite]" />
      ) : recording ? (
        <Square className="size-3.5 fill-current" />
      ) : (
        <Mic className="size-4" />
      )}
      {recording && <span className="tabular-nums">{voiceElapsed(elapsed)}</span>}
    </button>
  );
}
