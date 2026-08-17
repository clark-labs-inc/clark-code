import { useCallback, useEffect, useRef, useState } from "react";
import { Loader2, Mic, Square } from "lucide-react";
import { productModule } from "../product/productModule";
import {
  blobToBase64,
  cleanVoiceContentType,
  preferredVoiceMimeType,
  supportsVoiceRecording,
  startVoicePcmCapture,
  voiceCaptureMessage,
  voiceElapsed,
  voiceRecordingFilename,
  type VoicePcmCapture,
  type VoiceRecordingPhase,
} from "../lib/voiceNarration";
import { cn } from "../lib/cn";

export function ComposerVoiceButton({
  disabled,
  onTranscript,
  onError,
}: {
  disabled?: boolean;
  onTranscript: (text: string, state: "partial" | "final") => void;
  onError: (message: string) => void;
}) {
  const voice = productModule().voice;
  const transcriber = voice?.transcribe;
  const streamer = voice?.stream;
  const [phase, setPhase] = useState<VoiceRecordingPhase>("idle");
  const [elapsed, setElapsed] = useState(0);
  const [levels, setLevels] = useState(() => Array.from({ length: 18 }, () => 0.08));
  const [liveTranscript, setLiveTranscript] = useState("");
  const recorderRef = useRef<MediaRecorder | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const pcmCaptureRef = useRef<VoicePcmCapture | null>(null);
  const streamSessionRef = useRef<string | null>(null);
  const streamSendQueueRef = useRef<Promise<void>>(Promise.resolve());
  const streamFailureRef = useRef<unknown>(null);
  const streamLastTranscriptRef = useRef("");
  const chunksRef = useRef<Blob[]>([]);
  const startedAtRef = useRef(0);

  const stopStream = useCallback(() => {
    streamRef.current?.getTracks().forEach((track) => track.stop());
    streamRef.current = null;
  }, []);

  const stopPcmCapture = useCallback(async () => {
    const capture = pcmCaptureRef.current;
    pcmCaptureRef.current = null;
    if (capture) await capture.close();
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
    if (!transcriber && !streamer) {
      onError("Voice narration is unavailable in this Clark Code build.");
      return;
    }
    if (!supportsVoiceRecording(Boolean(streamer))) {
      onError("Voice recording is unavailable on this device.");
      return;
    }
    setPhase("connecting");
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          autoGainControl: true,
          channelCount: 1,
          echoCancellation: true,
          noiseSuppression: true,
        },
      });
      streamRef.current = stream;
      setElapsed(0);
      setLevels(Array.from({ length: 18 }, () => 0.08));
      setLiveTranscript("");

      if (streamer) {
        const session = await streamer.start();
        streamSessionRef.current = session.id;
        streamFailureRef.current = null;
        streamLastTranscriptRef.current = "";
        streamSendQueueRef.current = Promise.resolve();
        pcmCaptureRef.current = await startVoicePcmCapture(
          stream,
          (dataBase64) => {
            const id = streamSessionRef.current;
            if (!id || streamFailureRef.current) return;
            streamSendQueueRef.current = streamSendQueueRef.current.then(async () => {
              if (streamFailureRef.current) return;
              try {
                const result = await streamer.send(id, dataBase64);
                const text = result.text.trim();
                if (text && text !== streamLastTranscriptRef.current) {
                  streamLastTranscriptRef.current = text;
                  setLiveTranscript(text);
                  onTranscript(text, "partial");
                }
              } catch (error) {
                streamFailureRef.current = error;
              }
            });
          },
          (level) => setLevels((current) => [...current.slice(1), level]),
        );
        startedAtRef.current = Date.now();
        setPhase("recording");
        return;
      }

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
      recorderRef.current = recorder;
      startedAtRef.current = Date.now();
      setPhase("recording");
      recorder.start(1_000);
    } catch (error) {
      const sessionId = streamSessionRef.current;
      streamSessionRef.current = null;
      if (sessionId && streamer) await streamer.cancel(sessionId).catch(() => undefined);
      await stopPcmCapture().catch(() => undefined);
      stopStream();
      recorderRef.current = null;
      setPhase("idle");
      onError(voiceCaptureMessage(error));
    }
  }, [disabled, onError, onTranscript, phase, stopPcmCapture, stopStream, streamer, transcriber]);

  const finish = useCallback(async () => {
    if (phase !== "recording" || (!transcriber && !streamer)) return;
    setPhase("transcribing");
    try {
      if (streamer) {
        const sessionId = streamSessionRef.current;
        if (!sessionId) throw new Error("No active voice recording");
        await stopPcmCapture();
        await streamSendQueueRef.current;
        if (streamFailureRef.current) throw streamFailureRef.current;
        const result = await streamer.finish(sessionId);
        streamSessionRef.current = null;
        if (!result.text.trim()) throw new Error("No speech was detected.");
        onTranscript(result.text.trim(), "final");
        return;
      }

      if (!transcriber) throw new Error("Voice transcription is unavailable.");
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
      onTranscript(result.text.trim(), "final");
    } catch (error) {
      const sessionId = streamSessionRef.current;
      streamSessionRef.current = null;
      if (sessionId && streamer) await streamer.cancel(sessionId).catch(() => undefined);
      onError(error instanceof Error ? error.message : "Voice transcription failed.");
    } finally {
      await stopPcmCapture().catch(() => undefined);
      stopStream();
      recorderRef.current = null;
      chunksRef.current = [];
      streamLastTranscriptRef.current = "";
      setLiveTranscript("");
      setElapsed(0);
      setPhase("idle");
    }
  }, [onError, onTranscript, phase, stopPcmCapture, stopStream, stopToBlob, streamer, transcriber]);

  useEffect(() => {
    if (phase !== "recording") return;
    const timer = window.setInterval(() => setElapsed(Date.now() - startedAtRef.current), 250);
    return () => window.clearInterval(timer);
  }, [phase]);

  useEffect(() => () => {
    if (recorderRef.current?.state === "recording") recorderRef.current.stop();
    void stopPcmCapture().catch(() => undefined);
    const sessionId = streamSessionRef.current;
    streamSessionRef.current = null;
    if (sessionId && streamer) void streamer.cancel(sessionId).catch(() => undefined);
    stopStream();
  }, [stopPcmCapture, stopStream, streamer]);

  if (!transcriber && !streamer) return null;

  const recording = phase === "recording";
  const connecting = phase === "connecting";
  const transcribing = phase === "transcribing";
  return (
    <button
      type="button"
      onClick={() => recording ? void finish() : void start()}
      disabled={disabled || connecting || transcribing}
      aria-label={recording ? "Stop voice recording and transcribe" : "Start voice dictation"}
      title={recording ? `Stop and transcribe · ${voiceElapsed(elapsed)}` : "Narrate your idea"}
      className={cn(
        "flex h-8 shrink-0 items-center gap-1.5 rounded-full px-2 text-xs font-medium transition",
        recording
          ? "bg-accent-subtle text-accent ring-1 ring-accent/20"
          : "text-ink-muted hover:bg-accent-subtle hover:text-accent",
        (connecting || transcribing) && "text-accent",
      )}
    >
      {connecting || transcribing ? (
        <Loader2 className="size-4 animate-[spin_1s_linear_infinite]" />
      ) : recording ? (
        <Square className="size-3.5 fill-current" />
      ) : (
        <Mic className="size-4" />
      )}
      {recording && (
        <>
          {liveTranscript ? (
            <span className="max-w-48 truncate" aria-live="polite">{liveTranscript}</span>
          ) : (
            <span className="flex h-4 w-20 items-center justify-center gap-px" aria-hidden="true">
              {levels.map((level, index) => (
                <span
                  key={index}
                  className="w-0.5 rounded-full bg-current transition-[height] duration-fast"
                  style={{ height: `${Math.max(2, Math.round(level * 16))}px` }}
                />
              ))}
            </span>
          )}
          <span className="tabular-nums">{voiceElapsed(elapsed)}</span>
        </>
      )}
      {connecting && <span>Connecting…</span>}
      {transcribing && <span>Transcribing…</span>}
    </button>
  );
}
