// Reading agent-authored documents (markdown) off disk for inline rendering.
//
// The local agent writes documents into an app-managed workspace folder and
// emits them as artifacts whose `uri` is the absolute file path. The inline
// viewer reads the file's text on demand via a Tauri command (the file, not the
// snapshot, is the source of truth) and degrades gracefully everywhere else.

import { invoke } from "@tauri-apps/api/core";

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** True for a local filesystem path/URI we can read (not an http(s) URL). */
export function isLocalDocUri(uri?: string): boolean {
  if (!uri) return false;
  return !/^[a-z][a-z0-9+.-]*:\/\//i.test(uri) || uri.startsWith("file://");
}

/** Turn an artifact `uri` into a filesystem path (strips a `file://` scheme). */
function toPath(uri: string): string {
  return uri.startsWith("file://") ? decodeURIComponent(uri.slice("file://".length)) : uri;
}

/** Read a produced document's text from disk. Returns null when it can't be read
 *  inline (browser preview, a remote URL, or an unreadable/oversized file) — the
 *  caller falls back to an "Open" link. */
export async function readDocText(uri?: string): Promise<string | null> {
  if (!uri || !isTauri() || !isLocalDocUri(uri)) return null;
  try {
    return await invoke<string>("read_doc_text", { path: toPath(uri) });
  } catch {
    return null;
  }
}
