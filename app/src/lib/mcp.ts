// Client for the MCP probe Tauri command (a stateless "test connection" that
// connects each server, lists its tools, and returns status).

import { invoke } from "@tauri-apps/api/core";
import type { McpServerConfig } from "./mcpServers";

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export interface McpStatus {
  server: string;
  connected: boolean;
  tool_count: number;
  error?: string;
  tools: string[];
}

/** Probe servers; returns [] in the browser preview (desktop-only). */
export async function probeMcp(servers: McpServerConfig[]): Promise<McpStatus[]> {
  if (!isTauri() || servers.length === 0) return [];
  return invoke<McpStatus[]>("clark_mcp_probe", { servers });
}

export interface ClaudeSkill {
  name: string;
  description: string;
  path: string;
  scope: "project" | "personal";
}

/** MCP servers + skills discovered from an existing Claude Code setup in `cwd`
 *  (`.mcp.json`, `~/.claude.json`, `.claude/skills`). When `remote` is given,
 *  reads the remote host's `.claude` over the exec-server tunnel; otherwise the
 *  local disk. Empty in browser preview. */
export async function discoverClaude(
  cwd: string,
  remote?: { ws_url: string; token: string },
): Promise<{ mcp: McpServerConfig[]; skills: ClaudeSkill[] }> {
  if (!isTauri() || !cwd.trim()) return { mcp: [], skills: [] };
  return invoke("claude_discover", { cwd: cwd.trim(), remote: remote ?? null });
}
