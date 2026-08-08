// Client for the `list_commands` Tauri command — user-authored slash commands
// discovered from `.claude/commands/*.md` (project + personal).

import { invoke } from "@tauri-apps/api/core";

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export interface CustomCommand {
  name: string;
  description: string;
  body: string;
  scope: "project" | "personal";
}

/** Empty in browser preview (desktop-only) or when `cwd` isn't set yet. */
export async function listCustomCommands(
  cwd: string,
  remote?: { id: string },
): Promise<CustomCommand[]> {
  if (!isTauri() || !cwd.trim()) return [];
  try {
    return await invoke<CustomCommand[]>("list_commands", { cwd: cwd.trim(), remote: remote ?? null });
  } catch {
    return []; // no .claude/commands, unreadable, etc. — not fatal to the composer
  }
}
