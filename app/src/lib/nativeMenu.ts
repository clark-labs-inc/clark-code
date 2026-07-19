import { listen } from "@tauri-apps/api/event";

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function onNativeMenuEvent(event: string, handler: () => void): Promise<() => void> {
  if (!inTauri()) return () => {};
  return listen(event, handler);
}

/** Route the native Check for Updates menu item through the shared updater. */
export function onUpdateMenuRequested(handler: () => void): Promise<() => void> {
  return onNativeMenuEvent("update-menu-requested", handler);
}

/** Open the same Settings surface as the toolbar button and Cmd/Ctrl+, hotkey. */
export function onSettingsMenuRequested(handler: () => void): Promise<() => void> {
  return onNativeMenuEvent("settings-menu-requested", handler);
}
