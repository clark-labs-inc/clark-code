import type { SlashCommand } from "./slashCommands";

export interface PaletteCommandPresentation {
  label: string;
  hint?: string;
  searchText: string;
  prefill: string | null;
}

/** Adapt a slash command for the command palette. Prompt-style commands must
 * remain visibly named, searchable by their slash spelling, and insert their
 * editable body instead of becoming inert palette rows. */
export function paletteCommandPresentation(
  command: SlashCommand,
  actionLabel?: string,
): PaletteCommandPresentation {
  const promptStyle = command.body !== undefined;
  const label = promptStyle ? `/${command.name}` : actionLabel ?? command.hint;
  const hint = label !== command.hint ? command.hint : undefined;
  return {
    label,
    hint,
    searchText: `/${command.name} ${command.name} ${label} ${command.hint}`,
    prefill: promptStyle ? `${command.body} ` : null,
  };
}
