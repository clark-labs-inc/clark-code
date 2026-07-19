// Slash commands — quick actions typed in the composer (e.g. `/terminal`).
// They share the `@`-mention autocomplete popover; selecting one runs the action
// and clears the composer rather than inserting text. The same action set also
// backs the command palette, so it lives here once.

import { useSessionStore } from "../store/sessionStore";
import { conversationMarkdown } from "./transcript";

export interface SlashCommand {
  /** Command word, without the leading slash. */
  name: string;
  hint: string;
  /** Only applicable once a session is open (e.g. terminal, memory). */
  needsSession?: boolean;
  /** Built-in commands run an action and clear the composer. */
  run?: () => void;
  /** User-authored commands (`.claude/commands/*.md`) insert this into the
   *  composer instead of running an action — the user reviews before sending. */
  body?: string;
}

export function slashCommands(): SlashCommand[] {
  const s = () => useSessionStore.getState();
  return [
    { name: "new", hint: "Start a new conversation", run: () => s().endSession() },
    {
      name: "terminal",
      hint: "Toggle the terminal dock",
      needsSession: true,
      run: () => s().toggleTerminal(),
    },
    { name: "mcp", hint: "Manage MCP servers", run: () => s().setMcpOpen(true) },
    {
      name: "copy",
      hint: "Copy the conversation as Markdown",
      needsSession: true,
      run: () => {
        const md = conversationMarkdown(s().snapshot);
        if (!md) return;
        navigator.clipboard
          ?.writeText(md)
          .then(() => s().flashNotice("Conversation copied as Markdown"))
          .catch(() => s().flashNotice("Couldn't copy — clipboard unavailable"));
      },
    },
    {
      name: "share",
      hint: "Copy a public read-only link to this conversation",
      needsSession: true,
      run: () => void s().shareConversation(),
    },
    {
      name: "unshare",
      hint: "Stop sharing this conversation",
      needsSession: true,
      run: () => void s().unshareConversation(),
    },
    {
      name: "memory",
      hint: "View project memory",
      needsSession: true,
      run: () => s().toggleMemoryViewer(),
    },
    {
      name: "btw",
      hint: "Ask a side question without interrupting the run",
      needsSession: true,
      // Not a run-action: the composer intercepts `/btw <question>` on submit
      // and routes it to `askSideQuestion`. Insert the prefix so the user keeps
      // typing their question after the space.
      body: "/btw",
    },
  ];
}
