// Slash commands — quick actions typed in the composer (e.g. `/terminal`).
// They share the `@`-mention autocomplete popover; selecting one runs the action
// and clears the composer rather than inserting text. The same action set also
// backs the command palette, so it lives here once.

import { useSessionStore } from "../store/sessionStore";
import { conversationMarkdown } from "./transcript";

const SENTRY_SKILL = "$sentry:sentry";

export interface SubscriptionWorkflow {
  command:
    | "scout"
    | "security"
    | "security-diff"
    | "security-deep"
    | "scientist"
    | "rsi";
  skill?: string;
  label: string;
  value: string;
}

export const SUBSCRIPTION_WORKFLOWS: readonly SubscriptionWorkflow[] = [
  {
    command: "scout",
    skill: "scout:scout",
    label: "Scout",
    value: "Map your business systems end to end with evidence-backed coverage.",
  },
  {
    command: "security",
    skill: "security:security-scan",
    label: "Security Scan",
    value: "Find verified vulnerabilities across your repository.",
  },
  {
    command: "security-diff",
    skill: "security:security-diff",
    label: "Security Diff",
    value: "Catch security regressions in the exact change you are shipping.",
  },
  {
    command: "security-deep",
    skill: "security:security-deep",
    label: "Security Deep",
    value: "Run independent security passes until the review is saturated.",
  },
  {
    command: "scientist",
    label: "Scientist",
    value: "Run preregistered discovery programs with durable evidence and replication.",
  },
  {
    command: "rsi",
    label: "RSI",
    value: "Research and create evaluation worlds with reproducible counterexamples.",
  },
];

function subscriptionWorkflow(command: SubscriptionWorkflow["command"]): SubscriptionWorkflow {
  return SUBSCRIPTION_WORKFLOWS.find((candidate) => candidate.command === command)!;
}

export interface SlashCommand {
  /** Command word, without the leading slash. */
  name: string;
  hint: string;
  /** Only applicable once a session is open (e.g. terminal, memory). */
  needsSession?: boolean;
  /** Local coding-agent command; unavailable on cloud/ACP providers. */
  localOnly?: boolean;
  /** Paid, host-pinned workflow. It remains discoverable on Free, but submit
   *  must verify current Clark coverage before it can run. */
  subscriptionWorkflow?: SubscriptionWorkflow;
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
      name: "goal",
      hint: "Keep working autonomously until the objective is done",
      localOnly: true,
      // The provider recognizes this prefix deterministically and creates the
      // goal before the model begins the turn. Keep it in the composer so the
      // user can add the objective after choosing the command.
      body: "/goal",
    },
    {
      name: "compact",
      hint: "Summarize the conversation to free context space",
      needsSession: true,
      localOnly: true,
      run: () => void s().compactConversation(),
    },
    {
      name: "skills",
      hint: "Browse, select, install, and inspect skills",
      localOnly: true,
      body: "/skills",
    },
    {
      name: "scout",
      hint: "Map your business systems end to end",
      localOnly: true,
      body: "/scout",
      subscriptionWorkflow: subscriptionWorkflow("scout"),
    },
    {
      name: "security",
      hint: "Find verified repository vulnerabilities",
      localOnly: true,
      body: "/security",
      subscriptionWorkflow: subscriptionWorkflow("security"),
    },
    {
      name: "security-diff",
      hint: "Catch security regressions in this diff",
      localOnly: true,
      body: "/security-diff",
      subscriptionWorkflow: subscriptionWorkflow("security-diff"),
    },
    {
      name: "security-deep",
      hint: "Audit deeply with independent passes",
      localOnly: true,
      body: "/security-deep",
      subscriptionWorkflow: subscriptionWorkflow("security-deep"),
    },
    {
      name: "scientist",
      hint: "Turn an ambitious objective into a discovery program",
      localOnly: true,
      body: "/scientist",
      subscriptionWorkflow: subscriptionWorkflow("scientist"),
    },
    {
      name: "rsi",
      hint: "Research and create high-information evaluations",
      localOnly: true,
      body: "/rsi",
      subscriptionWorkflow: subscriptionWorkflow("rsi"),
    },
    {
      name: "sentry",
      hint: "Inspect current Sentry issues and production errors",
      localOnly: true,
      // Use the collision-safe bundled name. The explicit skill mention makes
      // selection deterministic even when a project also defines `sentry`.
      body: SENTRY_SKILL,
    },
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
      localOnly: true,
      // Not a run-action: the composer intercepts `/btw <question>` on submit
      // and routes it to `askSideQuestion`. Insert the prefix so the user keeps
      // typing their question after the space.
      body: "/btw",
    },
  ];
}

/** Return the objective for an exact `/goal` command, or `null` for ordinary
 * text. An empty string means the command is valid but still needs a goal. */
export function goalCommandObjective(text: string): string | null {
  const command = text.trimStart();
  if (!command.startsWith("/goal")) return null;
  const rest = command.slice("/goal".length);
  if (rest.length > 0 && !/^\s/.test(rest)) return null;
  return rest.trim();
}

/** Manual compaction is an exact control command with no inline arguments. */
export function isCompactCommand(text: string): boolean {
  return /^\s*\/compact\s*$/.test(text);
}

/** Return the question for an exact `/btw` command, or `null` for ordinary
 * text. An empty string means the command is valid but still needs a question. */
export function sideQuestionCommandQuestion(text: string): string | null {
  const command = text.trimStart();
  if (!command.startsWith("/btw")) return null;
  const rest = command.slice("/btw".length);
  if (rest.length > 0 && !/^\s/.test(rest)) return null;
  return rest.trim();
}

/** Expand directly typed prompt commands to collision-safe bundled skill
 * mentions. Autocomplete already inserts these forms; normalizing submit makes
 * typing a whole command by hand behave identically. */
export function expandPromptSlashCommand(text: string): string {
  const command = text.trimStart();
  const mappings = [
    { command: "/sentry", skill: SENTRY_SKILL },
  ];
  for (const mapping of mappings) {
    if (!command.startsWith(mapping.command)) continue;
    const rest = command.slice(mapping.command.length);
    if (rest.length > 0 && !/^\s/.test(rest)) return text;
    const leading = text.slice(0, text.length - command.length);
    return `${leading}${mapping.skill}${rest}`;
  }
  return text;
}

/** Identify a paid workflow regardless of whether it arrived as a slash
 * command, an expanded `$skill` mention, or a selected skill chip. */
export function subscriptionWorkflowForSubmission(
  text: string,
  selectedSkillNames: readonly string[] = [],
): SubscriptionWorkflow | null {
  const command = text.trimStart();
  const explicitSlash = SUBSCRIPTION_WORKFLOWS.find((workflow) => {
    const prefix = `/${workflow.command}`;
    if (!command.startsWith(prefix)) return false;
    const rest = command.slice(prefix.length);
    return rest.length === 0 || /^\s/.test(rest);
  });
  if (explicitSlash) return explicitSlash;

  const requestedSkills = [
    ...selectedSkillNames,
    ...Array.from(text.matchAll(/\$([A-Za-z0-9_:-]+)/g), (match) => match[1]),
  ].map((name) => name.toLowerCase());
  return SUBSCRIPTION_WORKFLOWS.find(
    (workflow) =>
      Boolean(workflow.skill)
      && requestedSkills.includes(workflow.skill!.toLowerCase()),
  ) ?? null;
}
