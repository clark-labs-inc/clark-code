export function newProjectDialogKeyboardIntent(key: string): "close" | "cycle_focus" | "none" {
  if (key === "Escape") return "close";
  if (key === "Tab") return "cycle_focus";
  return "none";
}
