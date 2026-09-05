export function newProjectDialogKeyboardIntent(key: string): "close" | "cycle_focus" | "none" {
  if (key === "Escape") return "close";
  if (key === "Tab") return "cycle_focus";
  return "none";
}

/** Refreshing saved hosts must not overwrite an in-progress folder choice. */
export function remoteFolderAfterHostRefresh(current: string, sameHost: boolean, defaultRoot: string): string {
  return sameHost ? current : defaultRoot;
}
