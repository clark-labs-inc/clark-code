// Plug-n-play attachment sources. Each is an independent hook that funnels
// picked files into one `onFiles` callback, so the composer stays agnostic to
// where files come from (drag-drop, paste, file picker, … add more freely).

import { useCallback, useEffect, useRef, useState } from "react";
import { combine } from "@atlaskit/pragmatic-drag-and-drop/combine";
import {
  dropTargetForExternal,
  monitorForExternal,
} from "@atlaskit/pragmatic-drag-and-drop/external/adapter";
import {
  containsFiles,
  getFiles,
} from "@atlaskit/pragmatic-drag-and-drop/external/file";
import { preventUnhandled } from "@atlaskit/pragmatic-drag-and-drop/prevent-unhandled";

const COMPOSER_FILE_DROP_TARGET = "composer-file-drop-target";

/** Pragmatic Drag and Drop target for files dragged from the desktop. The
 * returned ref belongs on the composer's outer region; file-picker and paste
 * sources remain equivalent non-drag alternatives. */
export function useFileDrop(onFiles: (files: File[]) => void) {
  const [dragging, setDragging] = useState(false);
  const [element, setElement] = useState<HTMLElement | null>(null);
  const callback = useRef(onFiles);
  callback.current = onFiles;

  useEffect(() => {
    if (!element) return;
    return dropTargetForExternal({
      element,
      canDrop: containsFiles,
      getData: () => ({ type: COMPOSER_FILE_DROP_TARGET }),
      getDropEffect: () => "copy",
      onDragEnter: () => setDragging(true),
      onDragLeave: () => setDragging(false),
      onDrop: ({ source }) => {
        setDragging(false);
        const files = getFiles({ source });
        if (files.length) callback.current(files);
      },
    });
  }, [element]);

  const dropTargetRef = useCallback((node: HTMLElement | null) => setElement(node), []);
  return { dragging, dropTargetRef };
}

/** Window-level Pragmatic Drag and Drop target. Tauri's native drag-drop is
 * disabled, so this both preserves drop-anywhere attachment and prevents an
 * unhandled desktop file from navigating the webview away. */
export function useWindowFileDropGuard(onFiles?: (files: File[]) => void) {
  const cb = useRef(onFiles);
  cb.current = onFiles;
  useEffect(() => {
    return combine(
      dropTargetForExternal({
        element: document.body,
        canDrop: containsFiles,
        getData: () => ({ type: "window-file-drop-target" }),
        getDropEffect: () => "copy",
        onDrop: ({ source, location }) => {
          const composerHandledDrop = location.current.dropTargets.some(
            (target) => target.data.type === COMPOSER_FILE_DROP_TARGET,
          );
          if (composerHandledDrop) return;
          const files = getFiles({ source });
          if (files.length) cb.current?.(files);
        },
      }),
      monitorForExternal({
        canMonitor: containsFiles,
        onDragStart: () => preventUnhandled.start(),
      }),
    );
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
