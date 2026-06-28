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
