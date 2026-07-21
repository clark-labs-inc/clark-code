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
  if (/^[a-z]:[\\/]/i.test(uri)) return true;
  return !/^[a-z][a-z0-9+.-]*:/i.test(uri) || uri.startsWith("file://");
}

/** Turn an artifact `uri` into a filesystem path (strips a `file://` scheme). */
export function toPath(uri: string): string {
  if (!uri.startsWith("file://")) return uri;
  let path = decodeURIComponent(uri.slice("file://".length));
  if (path.startsWith("localhost/")) path = path.slice("localhost".length);
  if (/^\/[a-z]:\//i.test(path)) path = path.slice(1);
  return path;
}

export type DocumentPreview =
  | { kind: "html"; html: string }
  | { kind: "pages"; preview_id: string; page_count: number };

export function isPreviewableDocument(uri?: string, title?: string, mimeType?: string): boolean {
  return (
    /(?:wordprocessingml|spreadsheetml|presentationml|opendocument|application\/pdf|text\/csv)/i.test(
      mimeType ?? "",
    ) ||
    /\.(?:docx?|odt|pdf|xlsx?|ods|csv|pptx?|odp)(?:[?#]|$)/i.test(uri ?? title ?? "")
  );
}

/** Render a local office document through Clark's bundled pure-Rust
 * libreoffice-rs engine. HTML remains inert inside the caller's sandbox. */
export async function readDocumentPreview(uri?: string): Promise<DocumentPreview | null> {
  if (!uri || !isTauri() || !isLocalDocUri(uri)) return null;
  try {
    return await invoke<DocumentPreview>("render_document_preview", { path: toPath(uri) });
  } catch {
    return null;
  }
}

/** Load one generated preview page through Tauri's raw-byte IPC path. */
export async function readDocumentPreviewPage(previewId: string, page: number): Promise<string> {
  const bytes = await invoke<ArrayBuffer>("read_document_preview_page", {
    previewId,
    page,
  });
  return URL.createObjectURL(new Blob([bytes], { type: "image/png" }));
}

export async function cleanupDocumentPreview(previewId: string): Promise<void> {
  await invoke("cleanup_document_preview", { previewId });
}

/** Read a produced document's text from disk. Returns null when it can't be read
 *  inline (browser preview, a remote URL, or an unreadable/oversized file) — the
 *  caller falls back to an "Open" link. */
export async function readDocText(uri?: string): Promise<string | null> {
  if (!uri) return null;
  if (!isLocalDocUri(uri)) {
    try {
      const response = await fetch(uri);
      if (!response.ok) return null;
      const declared = Number(response.headers.get("content-length") ?? 0);
      if (declared > 2 * 1024 * 1024) return null;
      const text = await response.text();
      return new TextEncoder().encode(text).byteLength <= 2 * 1024 * 1024 ? text : null;
    } catch {
      return null;
    }
  }
  if (!isTauri()) return null;
  try {
    return await invoke<string>("read_doc_text", { path: toPath(uri) });
  } catch {
    return null;
  }
}

/** Read a produced image's bytes from disk as a `data:` URL. Returns null when
 *  it can't be read inline (browser preview, a remote URL, or an
 *  unreadable/oversized/unsupported file) — the caller falls back to an "Open"
 *  link or leaves the image unrendered. */
export async function readImageDataUrl(uri?: string): Promise<string | null> {
  if (!uri || !isTauri() || !isLocalDocUri(uri)) return null;
  try {
    return await invoke<string>("read_image_data_url", { path: toPath(uri) });
  } catch {
    return null;
  }
}

/** A sensible `.md` filename derived from a document title (falls back to
 *  "document"). Strips path separators and collapses runs of whitespace. */
export function mdFileName(title?: string): string {
  const base = (title ?? "").trim() || "document";
  const cleaned = base.replace(/[/\\]+/g, " ").replace(/\s+/g, " ").trim();
  const name = cleaned || "document";
  return /\.(?:md|markdown|mdx)$/i.test(name) ? name : `${name}.md`;
}

/** A sensible `.pdf` filename derived from a Markdown artifact title. */
export function pdfFileName(title?: string): string {
  const markdownName = mdFileName(title);
  return markdownName.replace(/\.(?:md|markdown|mdx)$/i, ".pdf");
}

/** Save document text to disk. In the desktop app, opens the OS save dialog and
 *  writes the chosen path through the `save_doc_text` command (native download).
 *  Outside the app (browser preview), falls back to a Blob download. Returns
 *  whether the user actually saved (false if they cancelled the dialog). */
export async function saveDocText(text: string, title?: string): Promise<boolean> {
  const name = mdFileName(title);
  if (!isTauri()) {
    downloadBlob(text, name);
    return true;
  }
  const { save } = await import("@tauri-apps/plugin-dialog");
  const path = await save({
    title: "Save document",
    defaultPath: name,
    filters: [{ name: "Markdown", extensions: ["md", "markdown"] }],
  });
  if (!path) return false;
  await invoke("save_doc_text", { path, text });
  return true;
}

/** Export Markdown as a polished tagged PDF through libreoffice-pure. */
export async function saveDocPdf(
  text: string,
  title?: string,
  sourceUri?: string,
): Promise<boolean> {
  if (!isTauri()) return false;
  const { save } = await import("@tauri-apps/plugin-dialog");
  const path = await save({
    title: "Export document as PDF",
    defaultPath: pdfFileName(title),
    filters: [{ name: "PDF", extensions: ["pdf"] }],
  });
  if (!path) return false;
  const sourcePath = sourceUri && isLocalDocUri(sourceUri) ? toPath(sourceUri) : undefined;
  await invoke("export_markdown_pdf", { path, text, sourcePath });
  return true;
}

function downloadBlob(text: string, name: string): void {
  const blob = new Blob([text], { type: "text/markdown;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.rel = "noopener";
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}
