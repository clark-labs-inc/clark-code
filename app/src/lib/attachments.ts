// Attachment core: turn a picked File into a wire-ready PendingAttachment.
// Decoupled from the UI and from how it's sourced (drop / paste / picker) and
// from how it's sent (any provider). Images are downscaled for performance.

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

/** The minimal wire shape a provider ingests (mirrors Rust `PendingUpload`). */
export interface Upload {
  filename: string;
  content_type: string;
  data_base64: string;
}

export const MAX_ATTACHMENT_BYTES = 12 * 1024 * 1024;
const MAX_IMAGE_DIM = 1568;
const IMAGE_PASSTHROUGH_BYTES = 1_200_000;

export function toUpload(a: PendingAttachment): Upload {
  return { filename: a.filename, content_type: a.content_type, data_base64: a.data_base64 };
}

function arrayBufferToBase64(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

function loadImage(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = reject;
    img.src = url;
  });
}

/** Downscale a large image via canvas (perf); pass small ones through. */
async function processImage(file: File): Promise<PendingAttachment | null> {
  if (typeof document === "undefined") return null;
  const srcUrl = URL.createObjectURL(file);
  try {
    const img = await loadImage(srcUrl);
    const big = Math.max(img.width, img.height) > MAX_IMAGE_DIM;
    if (!big && file.size <= IMAGE_PASSTHROUGH_BYTES) {
      const buf = await file.arrayBuffer();
      return attach(file.name, file.type, arrayBufferToBase64(buf), file.size, srcUrl);
    }
    const scale = Math.min(1, MAX_IMAGE_DIM / Math.max(img.width, img.height));
    const w = Math.round(img.width * scale);
    const h = Math.round(img.height * scale);
    const canvas = document.createElement("canvas");
    canvas.width = w;
    canvas.height = h;
    canvas.getContext("2d")!.drawImage(img, 0, 0, w, h);
    const blob = await new Promise<Blob | null>((res) => canvas.toBlob(res, "image/webp", 0.85));
    URL.revokeObjectURL(srcUrl);
    if (!blob) return null;
    const buf = await blob.arrayBuffer();
    return attach(
      file.name.replace(/\.\w+$/, ".webp"),
      "image/webp",
      arrayBufferToBase64(buf),
      blob.size,
      URL.createObjectURL(blob),
    );
  } catch {
    URL.revokeObjectURL(srcUrl);
    return null;
  }
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

/** Convert a File into a PendingAttachment (downscaling images). */
export async function fileToAttachment(file: File): Promise<PendingAttachment> {
  if (file.type.startsWith("image/")) {
    const processed = await processImage(file);
    if (processed) return processed;
  }
  const buf = await file.arrayBuffer();
  return attach(file.name, file.type, arrayBufferToBase64(buf), file.size);
}

export function prettySize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
