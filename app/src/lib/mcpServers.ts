// MCP server configuration, persisted per-app. The enabled servers are sent to
// the local engine on connect (it spawns them and registers their tools), and
// can be probed independently for the settings UI.

export interface McpServer {
  id: string;
  /** Namespace shown in tool names (mcp_<name>_<tool>). */
  name: string;
  command: string;
  args: string[];
  env: Record<string, string>;
  enabled: boolean;
}

/** The wire shape the engine expects (extra.mcp_servers). */
export interface McpServerConfig {
  name: string;
  command: string;
  args: string[];
  env: Record<string, string>;
}

const KEY = "clark-desktop:mcp-servers";

export function loadMcpServers(): McpServer[] {
  try {
    const raw = localStorage.getItem(KEY);
    const list = raw ? (JSON.parse(raw) as McpServer[]) : [];
    return Array.isArray(list) ? list : [];
  } catch {
    return [];
  }
}

export function saveMcpServers(list: McpServer[]): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(list));
  } catch {
    /* quota — best effort */
  }
}

/** Enabled, complete servers in the engine's config shape. */
export function enabledMcpConfigs(list: McpServer[]): McpServerConfig[] {
  return list
    .filter((s) => s.enabled && s.name.trim() && s.command.trim())
    .map((s) => ({
      name: s.name.trim(),
      command: s.command.trim(),
      args: s.args.map((a) => a.trim()).filter(Boolean),
      env: s.env,
    }));
}

export function blankServer(): McpServer {
  return { id: crypto.randomUUID(), name: "", command: "", args: [], env: {}, enabled: true };
}

/** Built-in catalog of the most common MCP servers (all verified on npm).
 *  `make(cwd)` builds a ready-to-run server; `needs` flags anything the user
 *  must still fill in (a token, a connection string). */
export interface McpPreset {
  id: string;
  label: string;
  category: "Code" | "Web" | "Data" | "Knowledge";
  description: string;
  /** What the user still has to provide, if anything. */
  needs?: string;
  make: (cwd: string) => McpServer;
}

const npx = (name: string, command: string, args: string[], env: Record<string, string> = {}): McpServer => ({
  id: crypto.randomUUID(),
  name,
  command: "npx",
  args: ["-y", command, ...args],
  env,
  enabled: true,
});

export const MCP_PRESETS: McpPreset[] = [
  // --- Code & files ---
  {
    id: "filesystem",
    label: "Filesystem",
    category: "Code",
    description: "Read & write files in a folder",
    make: (cwd) => npx("filesystem", "@modelcontextprotocol/server-filesystem", [cwd || "."]),
  },
  {
    id: "github",
    label: "GitHub",
    category: "Code",
    description: "Issues, PRs, repos & code search",
    needs: "a GitHub token",
    make: () =>
      npx("github", "@modelcontextprotocol/server-github", [], {
        GITHUB_PERSONAL_ACCESS_TOKEN: "",
      }),
  },
  {
    id: "gitlab",
    label: "GitLab",
    category: "Code",
    description: "Projects, issues & merge requests",
    needs: "a GitLab token",
    make: () =>
      npx("gitlab", "@modelcontextprotocol/server-gitlab", [], {
        GITLAB_PERSONAL_ACCESS_TOKEN: "",
      }),
  },
  // --- Web & browser ---
  {
    id: "playwright",
    label: "Playwright",
    category: "Web",
    description: "Drive a real browser — navigate, click, scrape",
    make: () => npx("playwright", "@playwright/mcp@latest", []),
  },
  {
    id: "brave",
    label: "Brave Search",
    category: "Web",
    description: "Web & local search",
    needs: "a Brave API key",
    make: () =>
      npx("brave-search", "@modelcontextprotocol/server-brave-search", [], { BRAVE_API_KEY: "" }),
  },
  {
    id: "firecrawl",
    label: "Firecrawl",
    category: "Web",
    description: "Scrape & crawl sites into clean markdown",
    needs: "a Firecrawl API key",
    make: () => npx("firecrawl", "firecrawl-mcp", [], { FIRECRAWL_API_KEY: "" }),
  },
  {
    id: "context7",
    label: "Context7",
    category: "Web",
    description: "Up-to-date docs for any library",
    make: () => npx("context7", "@upstash/context7-mcp", []),
  },
  // --- Data ---
  {
    id: "postgres",
    label: "Postgres",
    category: "Data",
    description: "Query a Postgres database (read-only)",
    needs: "a connection string",
    make: () =>
      npx("postgres", "@modelcontextprotocol/server-postgres", [
        "postgresql://localhost/mydb",
      ]),
  },
  {
    id: "slack",
    label: "Slack",
    category: "Data",
    description: "Read & post to Slack channels",
    needs: "Slack tokens",
    make: () =>
      npx("slack", "@modelcontextprotocol/server-slack", [], {
        SLACK_BOT_TOKEN: "",
        SLACK_TEAM_ID: "",
      }),
  },
  // --- Knowledge & reasoning ---
  {
    id: "memory",
    label: "Memory",
    category: "Knowledge",
    description: "A persistent knowledge graph across sessions",
    make: () => npx("memory", "@modelcontextprotocol/server-memory", []),
  },
  {
    id: "sequential-thinking",
    label: "Sequential Thinking",
    category: "Knowledge",
    description: "Structured step-by-step reasoning",
    make: () => npx("thinking", "@modelcontextprotocol/server-sequential-thinking", []),
  },
];

/** Parse a textarea (one arg per line) into an args array. */
export function parseArgs(text: string): string[] {
  return text
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
}

/** Parse a "KEY=value" per line textarea into an env map. */
export function parseEnv(text: string): Record<string, string> {
  const env: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const eq = line.indexOf("=");
    if (eq <= 0) continue;
    const k = line.slice(0, eq).trim();
    const v = line.slice(eq + 1).trim();
    if (k) env[k] = v;
  }
  return env;
}

export function envToText(env: Record<string, string>): string {
  return Object.entries(env)
    .map(([k, v]) => `${k}=${v}`)
    .join("\n");
}
