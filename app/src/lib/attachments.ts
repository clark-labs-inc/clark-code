// Attachment core: turn a picked File into a wire-ready PendingAttachment.
// Decoupled from the UI and from how it's sourced (drop / paste / picker) and
// from how it's sent (any provider). Accepted files retain their original
// bytes; provider-specific media handling happens only after this boundary.

export interface PendingAttachment {
  id: string;
  filename: string;
  content_type: string;
  /** base64-encoded bytes, ready to send. */
  data_base64: string;
  size: number;
  /** object URL for an image thumbnail (revoke on remove). */
  previewUrl?: string;
}

/** Large clipboard text compacted in the composer but sent as ordinary text. */
export interface PendingPaste {
  id: string;
  placeholder: string;
  text: string;
  charCount: number;
}

/** The minimal wire shape a provider ingests (mirrors Rust `PendingUpload`). */
export interface Upload {
  filename: string;
  content_type: string;
  data_base64: string;
}

export type AttachmentKind = "text" | "image" | "audio" | "pdf" | "docx" | "binary";

export const LOCAL_ATTACHMENT_KINDS: AttachmentKind[] = ["text", "image", "pdf", "docx"];

export function attachmentKind(filename: string, contentType: string): AttachmentKind {
  const mime = contentType.toLowerCase();
  const name = filename.toLowerCase();
  if (mime.startsWith("image/")) return "image";
  if (mime.startsWith("audio/")) return "audio";
  if (mime === "application/pdf" || name.endsWith(".pdf")) return "pdf";
  if (
    mime === "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    || name.endsWith(".docx")
  ) return "docx";
  if (
    mime.startsWith("text/")
    || ["application/json", "application/xml", "application/javascript"].includes(mime)
  ) return "text";
  return "binary";
}

export const MAX_ATTACHMENT_BYTES = 12 * 1024 * 1024;
const LARGE_TEXT_PASTE_CHAR_THRESHOLD = 1_000;

/** Only text over 1,000 characters is compacted. */
export function shouldThumbnailPastedText(text: string): boolean {
  return text.trim().length > 0 && Array.from(text).length > LARGE_TEXT_PASTE_CHAR_THRESHOLD;
}

/** Create the unique display marker that stands in for a large paste. */
export function createPendingPaste(text: string, existing: PendingPaste[]): PendingPaste {
  const charCount = Array.from(text).length;
  const base = `[Pasted Content ${charCount} chars]`;
  const prefix = `${base} #`;
  let maxSuffix = 0;
  for (const paste of existing) {
    if (paste.placeholder === base) {
      maxSuffix = Math.max(maxSuffix, 1);
      continue;
    }
    if (paste.placeholder.startsWith(prefix)) {
      const suffixText = paste.placeholder.slice(prefix.length);
      if (/^\d+$/.test(suffixText)) {
        maxSuffix = Math.max(maxSuffix, Number.parseInt(suffixText, 10));
      }
    }
  }
  const placeholder = maxSuffix === 0 ? base : `${base} #${maxSuffix + 1}`;
  return {
    id:
      typeof crypto !== "undefined" && "randomUUID" in crypto
        ? crypto.randomUUID()
        : `paste-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    placeholder,
    text,
    charCount,
  };
}

/** Expand inline markers, or append chip-only pastes, into normal user text. */
export function expandPendingPastes(text: string, pastes: PendingPaste[]): string {
  let expanded = text;
  for (const paste of pastes) {
    if (expanded.includes(paste.placeholder)) {
      expanded = expanded.replace(paste.placeholder, paste.text);
    } else {
      expanded += `${expanded.trim() ? "\n\n" : ""}${paste.text}`;
    }
  }
  return expanded;
}

export function toUpload(a: PendingAttachment): Upload {
  return { filename: a.filename, content_type: a.content_type, data_base64: a.data_base64 };
}

function blobToBase64(blob: Blob): Promise<string> {
  if (typeof FileReader === "undefined") {
    // Non-browser test/dev runtimes do not expose FileReader. Keep a bounded
    // compatibility path there; the Tauri WebView uses the native reader
    // below and avoids this extra binary string allocation.
    return blob.arrayBuffer().then((buffer) => {
      const bytes = new Uint8Array(buffer);
      let binary = "";
      const chunk = 0x8000;
      for (let index = 0; index < bytes.length; index += chunk) {
        binary += String.fromCharCode(...bytes.subarray(index, index + chunk));
      }
      return btoa(binary);
    });
  }

  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error("Could not read attachment"));
    reader.onload = () => {
      if (typeof reader.result !== "string") {
        reject(new Error("Could not encode attachment"));
        return;
      }
      const separator = reader.result.indexOf(",");
      if (separator < 0) {
        reject(new Error("Attachment encoding was malformed"));
        return;
      }
      resolve(reader.result.slice(separator + 1));
    };
    // Let the WebView's native file reader encode the blob. The previous JS
    // path built a second byte-sized binary string before calling `btoa`,
    // which could block the composer and briefly triple memory for a 12 MB PDF.
    reader.readAsDataURL(blob);
  });
}

function attach(
  filename: string,
  content_type: string,
  data_base64: string,
  size: number,
  previewUrl?: string,
): PendingAttachment {
  return {
    id:
      typeof crypto !== "undefined" && "randomUUID" in crypto
        ? crypto.randomUUID()
        : `att-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    filename: filename || "file",
    content_type: content_type || "application/octet-stream",
    data_base64,
    size,
    previewUrl,
  };
}

/** Convert a File into a byte-for-byte PendingAttachment. */
export async function fileToAttachment(file: File): Promise<PendingAttachment> {
  const previewUrl = file.type.startsWith("image/")
    && typeof URL !== "undefined"
    && typeof URL.createObjectURL === "function"
    ? URL.createObjectURL(file)
    : undefined;
  return attach(file.name, file.type, await blobToBase64(file), file.size, previewUrl);
}

export function prettySize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
