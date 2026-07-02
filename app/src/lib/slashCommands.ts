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
  run: () => void;
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
        const { peek, snapshot } = s();
        const md = conversationMarkdown(peek ? peek.snapshot : snapshot);
        if (md) void navigator.clipboard?.writeText(md);
      },
    },
    {
      name: "memory",
      hint: "View project memory",
      needsSession: true,
      run: () => s().toggleMemoryViewer(),
    },
  ];
}
