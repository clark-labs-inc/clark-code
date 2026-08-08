import { open as shellOpen } from "@tauri-apps/plugin-shell";

/** Open a trusted URL in the system browser or a new preview tab. */
export async function openExternal(url: string): Promise<void> {
  try {
    await shellOpen(url);
  } catch {
    if (typeof window !== "undefined") window.open(url, "_blank", "noopener");
  }
}
