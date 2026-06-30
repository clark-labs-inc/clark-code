// Settings for the "Local coding" provider — the OpenCode-style mode where the
// agent loop runs on this machine, the model is reached over an
// OpenAI-compatible API (GLM-5.2 via OpenRouter by default), and research is
// delegated to Clark's sandbox.
//
// Persisted in localStorage; the model API key is the only secret and never
// leaves the device except in requests to the configured endpoint.

import type { ConnectConfig } from "../core-bridge/bridge";
import { loadAllowlist, loadDenylist } from "./commandPolicy";
import { loadMcpServers, enabledMcpConfigs } from "./mcpServers";
import type { RemoteTargetConfig } from "./ssh";

const KEY = "clark-desktop:local-agent";

export interface LocalAgentSettings {
  /** Absolute path to the project the agent edits. */
  cwd: string;
  /** Clark model id (see `GET /v1/models`). */
  model: string;
  /** Clark Platform API key (`ck_live_…`). The only credential. */
  apiKey: string;
}

export const DEFAULT_LOCAL_SETTINGS: LocalAgentSettings = {
  cwd: "",
  model: "clark-code",
  apiKey: "",
};

export function loadLocalSettings(): LocalAgentSettings {
  try {
    const raw = localStorage.getItem(KEY);
    const merged = raw
      ? { ...DEFAULT_LOCAL_SETTINGS, ...(JSON.parse(raw) as Partial<LocalAgentSettings>) }
      : { ...DEFAULT_LOCAL_SETTINGS };
    return migrate(merged);
  } catch {
    return { ...DEFAULT_LOCAL_SETTINGS };
  }
}

// Older installs saved a raw OpenRouter model id (e.g. "z-ai/glm-5.2"). The
// production Clark API only accepts Clark tier ids, which never contain "/", so
// coerce any such stale value to the coding default — otherwise the request 400s
// with "Unknown Clark model tier".
function migrate(s: LocalAgentSettings): LocalAgentSettings {
  return s.model.includes("/") ? { ...s, model: DEFAULT_LOCAL_SETTINGS.model } : s;
}

export function saveLocalSettings(settings: LocalAgentSettings): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(settings));
  } catch {
    // Non-fatal: settings just won't persist across restarts.
  }
}

/**
 * Build the `connect` config the native coding provider expects. Everything
 * routes through the production Clark Platform API; the only inputs are the
 * project folder, an optional model id, and the `ck_live_` key. Research uses
 * the same key automatically.
 */
export function localConnectConfig(
  s: LocalAgentSettings,
  remote?: RemoteTargetConfig,
): ConnectConfig {
  // For a remote project the root lives on the remote host; tool I/O runs there
  // over the exec-server. The command policy is still keyed by the project path.
  const project = (remote ? remote.cwd : s.cwd).trim();
  return {
    cwd: remote ? undefined : project || undefined,
    auth_token: s.apiKey.trim() || undefined,
    extra: {
      model: s.model.trim() || "clark-code",
      // Per-project shell-command policy the engine consults to skip / block the gate.
      command_allowlist: loadAllowlist(project),
      command_denylist: loadDenylist(project),
      // MCP servers to spawn + expose as tools.
      mcp_servers: enabledMcpConfigs(loadMcpServers()),
      // When present, the provider runs this session's tools on the remote host.
      ...(remote ? { remote } : {}),
    },
  };
}

/** Whether the settings are complete enough to start a session. The Clark Code
 *  API key is provisioned automatically on sign-in, so it isn't required here. */
export function localSettingsReady(s: LocalAgentSettings): string | null {
  if (!s.cwd.trim()) return "Choose a project folder.";
  return null;
}

const RECENTS_KEY = "clark-desktop:recent-projects";
const MAX_RECENTS = 8;

/** Most-recently-used project folders, newest first. */
export function loadRecentProjects(): string[] {
  try {
    const raw = localStorage.getItem(RECENTS_KEY);
    if (!raw) return [];
    const list = JSON.parse(raw) as unknown;
    return Array.isArray(list) ? list.filter((p): p is string => typeof p === "string") : [];
  } catch {
    return [];
  }
}

/** Push `path` to the front of the recents list (de-duped), and persist. */
export function addRecentProject(path: string): string[] {
  const clean = path.trim();
  if (!clean) return loadRecentProjects();
  const next = [clean, ...loadRecentProjects().filter((p) => p !== clean)].slice(0, MAX_RECENTS);
  try {
    localStorage.setItem(RECENTS_KEY, JSON.stringify(next));
  } catch {
    // Non-fatal.
  }
  return next;
}

/** The last path segment of a folder, for compact display. */
export function projectName(path: string): string {
  const cleaned = path.replace(/[/\\]+$/, "");
  const parts = cleaned.split(/[/\\]/);
  return parts[parts.length - 1] || cleaned || path;
}
