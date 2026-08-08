import { invoke } from "@tauri-apps/api/core";
import { getBridge } from "../core-bridge/bridge";

const SAFE_MARKDOWN_PROTOCOL = /^(https?|ircs?|mailto|xmpp)$/i;

function decoded(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

function withoutLocationSuffix(value: string): string {
  const index = value.search(/[?#]/);
  return index < 0 ? value : value.slice(0, index);
}

function fileUrlPath(href: string): string | null {
  try {
    const url = new URL(href);
    if (url.protocol !== "file:") return null;
    const host = url.hostname && url.hostname !== "localhost" ? `//${url.hostname}` : "";
    let path = decoded(url.pathname);
    if (/^\/[a-z]:\//i.test(path)) path = path.slice(1);
    return `${host}${path}`;
  } catch {
    return decoded(href.slice("file://".length));
  }
}

function isAbsolutePath(value: string): boolean {
  return value.startsWith("/") || value.startsWith("\\\\") || /^[a-z]:[\\/]/i.test(value);
}

function joinProjectPath(cwd: string, relative: string): string {
  if (!cwd) return relative;
  const separator = cwd.includes("\\") && !cwd.includes("/") ? "\\" : "/";
  return `${cwd.replace(/[/\\]+$/, "")}${separator}${relative.replace(/^[/\\]+/, "")}`;
}

/** Resolve a Markdown destination to a local path. Returns null for web URLs,
 * anchors, and unsupported schemes. Scheme-less destinations are files rooted
 * at the active project, matching how coding agents author deliverable links. */
export function localPathFromHref(href: string | undefined, cwd: string): string | null {
  const value = href?.trim();
  if (!value || value.startsWith("#")) return null;
  if (value.toLowerCase().startsWith("file://")) return fileUrlPath(value);
  if (value.startsWith("//")) return null;
  if (/^[a-z]:[\\/]/i.test(value) || value.startsWith("\\\\")) return decoded(value);
  if (/^[a-z][a-z0-9+.-]*:/i.test(value)) return null;

  const path = decoded(withoutLocationSuffix(value));
  if (!path) return null;
  return isAbsolutePath(path) || path.startsWith("~") ? path : joinProjectPath(cwd, path);
}

/** Preserve filesystem destinations while rejecting active or unknown URL
 * schemes. This policy belongs to the agent rather than to a Markdown renderer. */
export function markdownUrlTransform(value: string): string {
  if (localPathFromHref(value, "") !== null) return value;
  const colon = value.indexOf(":");
  const questionMark = value.indexOf("?");
  const numberSign = value.indexOf("#");
  const slash = value.indexOf("/");
  const isRelative = colon === -1
    || (slash !== -1 && colon > slash)
    || (questionMark !== -1 && colon > questionMark)
    || (numberSign !== -1 && colon > numberSign);
  return isRelative || SAFE_MARKDOWN_PROTOCOL.test(value.slice(0, colon)) ? value : "";
}

export function localFileName(path: string): string {
  const clean = path.replace(/[\\/]+$/, "");
  return clean.split(/[\\/]/).pop() || "download";
}

export async function openLocalPath(path: string, reveal = false): Promise<void> {
  const bridge = await getBridge();
  if (!bridge.openPath) throw new Error("Opening local files is unavailable.");
  await bridge.openPath(path, reveal);
}

/** Save a copy of an existing local file through the OS save dialog. */
export async function saveLocalFileCopy(path: string): Promise<boolean> {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) {
    throw new Error("Saving local files is available in the desktop app.");
  }
  const { save } = await import("@tauri-apps/plugin-dialog");
  const destination = await save({
    title: "Save a copy",
    defaultPath: localFileName(path),
  });
  if (!destination) return false;
  await invoke("copy_local_file", { source: path, destination });
  return true;
}
