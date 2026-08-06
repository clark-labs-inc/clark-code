import { invoke } from "@tauri-apps/api/core";

type FrontendDiagnosticKind = "exception" | "rejection" | "boundary";

interface FrontendDiagnostic {
  kind: FrontendDiagnosticKind;
  name?: string;
  reference: string;
  stack_frames?: string;
  source?: string;
  line?: number;
  column?: number;
  component_stack?: string;
}

let installed = false;

export function privacySafeDiagnosticReference(name: string, message: string, stack = ""): string {
  const source = `${name}\n${message}\n${stack}`;
  let hash = 0x811c9dc5;
  for (let index = 0; index < source.length; index += 1) {
    hash ^= source.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return `DESKTOP-${(hash >>> 0).toString(16).toUpperCase().padStart(8, "0")}`;
}

export function privacySafeStackFrames(error: Error): string | undefined {
  const frames = error.stack?.split("\n").slice(1).join("\n").trim();
  return frames ? frames.slice(0, 32_768) : undefined;
}

function safeSource(source?: string): string | undefined {
  if (!source) return undefined;
  try {
    return new URL(source, window.location.href).pathname.slice(0, 2_048);
  } catch {
    return source.split(/[?#]/, 1)[0]?.slice(0, 2_048);
  }
}

function send(input: FrontendDiagnostic): void {
  if (!import.meta.env.DEV) return;
  void invoke("capture_frontend_diagnostic", { input }).catch(() => {
    // Capture is optional and must never create a secondary UI failure.
  });
}

export function captureFrontendException(
  error: Error,
  options: {
    kind?: FrontendDiagnosticKind;
    reference?: string;
    componentStack?: string;
  } = {},
): void {
  if (!import.meta.env.DEV) return;
  send({
    kind: options.kind ?? "exception",
    name: error.name,
    reference: options.reference ?? privacySafeDiagnosticReference(error.name, error.message, error.stack),
    ...(privacySafeStackFrames(error) ? { stack_frames: privacySafeStackFrames(error) } : {}),
    ...(options.componentStack ? { component_stack: options.componentStack.slice(0, 16_384) } : {}),
  });
}

export function installLocalCapture(): void {
  if (!import.meta.env.DEV) return;
  if (installed) return;
  installed = true;
  window.addEventListener("error", (event) => {
    const error = event.error instanceof Error ? event.error : new Error("WindowError");
    send({
      kind: "exception",
      name: error.name,
      reference: privacySafeDiagnosticReference(error.name, error.message, error.stack),
      ...(privacySafeStackFrames(error) ? { stack_frames: privacySafeStackFrames(error) } : {}),
      ...(safeSource(event.filename) ? { source: safeSource(event.filename) } : {}),
      ...(event.lineno ? { line: event.lineno } : {}),
      ...(event.colno ? { column: event.colno } : {}),
    });
  });
  window.addEventListener("unhandledrejection", (event) => {
    const error = event.reason instanceof Error ? event.reason : new Error("NonErrorRejection");
    captureFrontendException(error, { kind: "rejection" });
  });
}
