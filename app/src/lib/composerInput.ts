import type { SlashCommand } from "./slashCommands";
import type { SkillCatalogEntry } from "../core-bridge/bridge";

/** What the user is mid-typing at the caret: an `@file` mention (anywhere) or a
 * `/command` (only at the very start of the message). */
export interface ComposerTrigger {
  type: "@" | "/" | "$";
  query: string;
  /** Index of the trigger character in the text. */
  start: number;
}

export type ComposerSuggestion =
  | { kind: "parent_directory_menu" }
  | { kind: "parent_directory_picker" }
  | { kind: "parent_directory"; path: string; root: string }
  | { kind: "directory"; path: string }
  | { kind: "file"; path: string }
  | { kind: "slash"; cmd: SlashCommand }
  | { kind: "skill"; skill: SkillCatalogEntry };

export function detectComposerTrigger(text: string, caret: number): ComposerTrigger | null {
  for (let i = caret - 1; i >= 0; i--) {
    const ch = text[i];
    // Slashes inside an `@` path are query content, not failed slash-command
    // triggers. Keep scanning until the leading `@` (or whitespace) decides.
    if (ch === "/" && i !== 0) continue;
    if (ch === "@" || ch === "/" || ch === "$") {
      const before = i === 0 ? "" : text[i - 1];
      if (i !== 0 && !/\s/.test(before)) return null;
      const query = text.slice(i + 1, caret);
      if (/\s/.test(query)) return null;
      return { type: ch, query, start: i };
    }
    if (/\s/.test(ch)) return null;
  }
  return null;
}

interface ComposerSubmissionInput {
  hasContent: boolean;
  hasSession: boolean;
  connecting: boolean;
  activeProvider: string | null;
  projectMode: "local" | "remote";
  localCwd: string;
  startBlocked: string | null;
  canPickProjectFolder: boolean;
  usesConversationWorkspace?: boolean;
}

export function composerSubmissionState(input: ComposerSubmissionInput) {
  const needsProjectFolder =
    !input.hasSession &&
    input.activeProvider === "local" &&
    input.projectMode === "local" &&
    !input.usesConversationWorkspace &&
    !input.localCwd.trim();
  const canResolveBlockedStart = needsProjectFolder && input.canPickProjectFolder;
  const conversationWorkspaceResolvesStart = Boolean(input.usesConversationWorkspace)
    && input.projectMode === "local";
  const canSubmit =
    input.hasContent &&
    (input.hasSession ||
      (!input.connecting &&
        input.activeProvider !== null &&
        (!input.startBlocked || canResolveBlockedStart || conversationWorkspaceResolvesStart)));

  return { canSubmit, shouldPickProjectFolder: canSubmit && needsProjectFolder };
}
