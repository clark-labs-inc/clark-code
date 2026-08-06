// Client for the MCP probe Tauri command (a stateless "test connection" that
// connects each server, lists its tools, and returns status).

import { invoke } from "@tauri-apps/api/core";
import type { McpServer, McpServerConfig } from "./mcpServers";

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
  if (servers.length === 0) return [];
  if (!isTauri()) throw new Error("Connection testing is available in the desktop app.");
  return invoke<McpStatus[]>("clark_mcp_probe", { servers });
}

/** Replace the signed-in account's native encrypted MCP credential set. Blank
 * values retain an existing secret; omitted servers and names are deleted. */
export async function syncMcpCredentials(servers: McpServer[]): Promise<void> {
  if (!isTauri()) return;
  await invoke("clark_mcp_credentials_sync", {
    servers: servers.map((server) => ({ id: server.id, environment: server.env })),
  });
}

export type MigrationSource = "claude" | "openai";

export interface MigratedSkill {
  name: string;
  description: string;
  path: string;
  scope: "project" | "personal";
  source: MigrationSource;
}

export interface MigratedInstruction {
  path: string;
  scope: "project" | "personal";
  source: MigrationSource;
}

export interface AgentMigrationDiscovery {
  source: MigrationSource;
  mcp: McpServerConfig[];
  skills: MigratedSkill[];
  instructions: MigratedInstruction[];
}

/** Read-only discovery from compatible coding agents on the project executor. */
export async function discoverAgentSetups(
  cwd: string,
  remote?: { id: string },
): Promise<AgentMigrationDiscovery[]> {
  if (!isTauri() || !cwd.trim()) return [];
  return invoke("external_agent_discover", { cwd: cwd.trim(), remote: remote ?? null });
}

/** Merge only missing names. Existing Clark configuration always wins. */
export function mergeDiscoveredMcp(
  existing: McpServer[],
  discovered: McpServerConfig[],
  createId: () => string = () => crypto.randomUUID(),
): { servers: McpServer[]; added: number } {
  const names = new Set(existing.map((server) => server.name.trim()).filter(Boolean));
  const added: McpServer[] = [];
  for (const server of discovered) {
    const name = server.name.trim();
    const command = server.command.trim();
    if (!name || !command || names.has(name)) continue;
    names.add(name);
    added.push({
      id: createId(),
      name,
      command,
      args: server.args ?? [],
      env: server.env ?? {},
      enabled: true,
    });
  }
  return { servers: [...existing, ...added], added: added.length };
}
