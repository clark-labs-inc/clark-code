// Native directory picker for choosing the local coding project root.
//
// Uses the Tauri dialog plugin when running in the desktop app; returns null in
// the browser preview (where the form falls back to a manual path field).

export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Open the OS folder picker. Returns the chosen absolute path, or null. */
export async function pickFolder(defaultPath?: string): Promise<string | null> {
  if (!inTauri()) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selected = await open({
    directory: true,
    multiple: false,
    title: "Select project folder",
    defaultPath: defaultPath || undefined,
  });
  return typeof selected === "string" ? selected : null;
}
