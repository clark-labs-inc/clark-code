// Plug-n-play attachment sources. Each is an independent hook that funnels
// picked files into one `onFiles` callback, so the composer stays agnostic to
// where files come from (drag-drop, paste, file picker, … add more freely).

import { useCallback, useEffect, useRef, useState, type DragEvent } from "react";

/** Drag-and-drop source: returns drag state + handlers to spread on a region. */
export function useFileDrop(onFiles: (files: File[]) => void) {
  const [dragging, setDragging] = useState(false);
  const depth = useRef(0);

  const onDragEnter = useCallback((e: DragEvent) => {
    if (!Array.from(e.dataTransfer.types).includes("Files")) return;
    e.preventDefault();
    depth.current += 1;
    setDragging(true);
  }, []);
  const onDragOver = useCallback((e: DragEvent) => {
    if (Array.from(e.dataTransfer.types).includes("Files")) e.preventDefault();
  }, []);
  const onDragLeave = useCallback((e: DragEvent) => {
    e.preventDefault();
    depth.current = Math.max(0, depth.current - 1);
    if (depth.current === 0) setDragging(false);
  }, []);
  const onDrop = useCallback(
    (e: DragEvent) => {
      e.preventDefault();
      depth.current = 0;
      setDragging(false);
      const files = Array.from(e.dataTransfer.files);
      if (files.length) onFiles(files);
    },
    [onFiles],
  );

  return { dragging, handlers: { onDragEnter, onDragOver, onDragLeave, onDrop } };
}

/** Window-level safety net for OS file drops. Tauri's native drag-drop is
 *  disabled (so the composer's HTML5 drop target works), and with it gone an
 *  unhandled file drop would navigate the webview to the file — wiping the UI.
 *  Swallow every file drop no other target handled (React handlers run before
 *  this window listener, so `defaultPrevented` marks the composer's drops) and
 *  optionally forward it, giving drop-anywhere-in-the-window attach. */
export function useWindowFileDropGuard(onFiles?: (files: File[]) => void) {
  const cb = useRef(onFiles);
  cb.current = onFiles;
  useEffect(() => {
    const hasFiles = (e: globalThis.DragEvent) =>
      Array.from(e.dataTransfer?.types ?? []).includes("Files");
    const onDragOver = (e: globalThis.DragEvent) => {
      if (hasFiles(e)) e.preventDefault();
    };
    const onDrop = (e: globalThis.DragEvent) => {
      if (!hasFiles(e) || e.defaultPrevented) return;
      e.preventDefault();
      const files = Array.from(e.dataTransfer?.files ?? []);
      if (files.length) cb.current?.(files);
    };
    window.addEventListener("dragover", onDragOver);
    window.addEventListener("drop", onDrop);
    return () => {
      window.removeEventListener("dragover", onDragOver);
      window.removeEventListener("drop", onDrop);
    };
  }, []);
}

/** Paste source: picks up files/images pasted anywhere while enabled. */
export function usePaste(onFiles: (files: File[]) => void, enabled = true) {
  useEffect(() => {
    if (!enabled) return;
    const handler = (e: ClipboardEvent) => {
      const files = Array.from(e.clipboardData?.files ?? []);
      if (files.length) onFiles(files);
    };
    window.addEventListener("paste", handler);
    return () => window.removeEventListener("paste", handler);
  }, [onFiles, enabled]);
}
