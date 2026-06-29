// Native OS notifications, desktop-only. The whole point of a native app over a
// browser tab: when a run finishes, fails, or blocks on approval while the user
// has tabbed away, ping them. No-op in the browser preview, and never fires when
// the window is already focused (don't interrupt someone who's watching).

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

let permission: "granted" | "denied" | "default" | null = null;

export async function notify(title: string, body: string): Promise<void> {
  if (!inTauri()) return;
  if (typeof document !== "undefined" && document.hasFocus()) return;
  try {
    const mod = await import("@tauri-apps/plugin-notification");
    if (permission === null) {
      permission = (await mod.isPermissionGranted())
        ? "granted"
        : await mod.requestPermission();
    }
    if (permission !== "granted") return;
    mod.sendNotification({ title, body });
  } catch {
    /* notifications unavailable — non-fatal */
  }
}
