// Thin client for the native PTY commands (src-tauri/src/terminal.rs). Only the
// desktop (Tauri) build has a real terminal; in the browser preview these are
// no-ops and `isTauri()` is false so the UI can show a hint instead.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export interface TermDataEvent {
  id: string;
  /** base64 of the raw PTY bytes. */
  chunk: string;
}

export function openTerminal(
  id: string,
  cwd: string | undefined,
  cols: number,
  rows: number,
): Promise<void> {
  return invoke("terminal_open", { id, cwd, cols, rows });
}

export function writeTerminal(id: string, data: string): Promise<void> {
  return invoke("terminal_write", { id, data });
}

export function resizeTerminal(id: string, cols: number, rows: number): Promise<void> {
  return invoke("terminal_resize", { id, cols, rows });
}

export function closeTerminal(id: string): Promise<void> {
  return invoke("terminal_close", { id });
}

/** Subscribe to a terminal's output stream (decoded to bytes). */
export function onTerminalData(
  id: string,
  onBytes: (bytes: Uint8Array) => void,
): Promise<UnlistenFn> {
  return listen<TermDataEvent>("terminal://data", (e) => {
    if (e.payload.id !== id) return;
    onBytes(base64ToBytes(e.payload.chunk));
  });
}

/** Subscribe to a terminal's shell-exit signal. */
export function onTerminalExit(id: string, onExit: () => void): Promise<UnlistenFn> {
  return listen<string>("terminal://exit", (e) => {
    if (e.payload === id) onExit();
  });
}

function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}
