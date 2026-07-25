// Settings for the "Local coding" provider — the OpenCode-style mode where the
// agent loop runs on this machine, the model is reached over an
// OpenAI-compatible API (GLM 5.2 via Clark by default), and research is
// delegated to Clark's sandbox.
//
// Persisted in localStorage; the model API key is the only secret and never
// leaves the device except in requests to Clark's API.

import type { ConnectConfig } from "../core-bridge/bridge";
import { loadAllowlist, loadDenylist } from "./commandPolicy";
import { loadMcpServers, enabledMcpConfigs } from "./mcpServers";
import type { RemoteTargetConfig } from "./ssh";
import { projectKnowledgeEnabled } from "./repositoryKnowledge";

const KEY = "clark-desktop:local-agent";
const env = import.meta.env as Record<string, string | undefined>;

export interface LocalAgentSettings {
  /** Absolute path to the project the agent edits. */
  cwd: string;
  /** Clark model id (see `GET /v1/models`). */
  model: string;
  /** Reasoning effort sent with each request ("" = the model's default). */
  reasoningEffort: string;
  /** Clark Platform API key (`ck_live_…`). The only credential. */
  apiKey: string;
  /** Stable signed-in account binding for `apiKey`. Empty means the key came
   *  from a legacy build and must be re-provisioned before reuse. */
  apiKeyOwner?: string;
  /** Opt-in local control of ordinary desktop applications. Native support is
   *  platform-gated again by the host and by OS privacy permissions. */
  computerUseEnabled?: boolean;
}

export const DEFAULT_LOCAL_SETTINGS: LocalAgentSettings = {
  cwd: "",
  model: "clark-code",
  reasoningEffort: "",
  apiKey: "",
  apiKeyOwner: "",
  computerUseEnabled: false,
};

/** Reasoning effort ids accepted by the OpenRouter models behind Clark Code. */
export type ReasoningEffortId =
  | ""
  | "max"
  | "xhigh"
  | "high"
  | "medium"
  | "low"
  | "minimal";

/** All effort labels used across the model-specific selectors. `max` and
 *  `xhigh` are distinct OpenRouter wire values but share the same product label. */
export const REASONING_EFFORTS = [
  { id: "", label: "Auto" },
  { id: "max", label: "Max" },
  { id: "xhigh", label: "Max" },
  { id: "high", label: "High" },
  { id: "medium", label: "Medium" },
  { id: "low", label: "Low" },
  { id: "minimal", label: "Minimal" },
] as const;

/** The coding models the composer picker offers (clark-code tier options).
 *  `reasoningEfforts` mirrors OpenRouter's `GET /api/v1/models` reasoning
 *  metadata. An empty list means the model reasons but exposes no effort knob. */
export const CODING_MODELS = [
  {
    id: "clark-code",
    label: "GLM 5.2",
    hint: "Deep reasoning · default",
    priceTier: 3,
    reasoningEfforts: ["", "xhigh", "high"],
    defaultReasoningEffort: "",
  },
  {
    id: "clark-code:minimax_m3",
    label: "MiniMax M3",
    hint: "Efficient tool calling · vision · 1M context",
    priceTier: 1,
    reasoningEfforts: [],
    defaultReasoningEffort: "",
  },
  {
    id: "clark-code:kimi_k3",
    label: "Kimi K3",
    hint: "Long-horizon coding · vision · 1M context",
    priceTier: 5,
    reasoningEfforts: ["max"],
    defaultReasoningEffort: "max",
  },
  {
    id: "clark-code:kimi_k27_code",
    label: "Kimi K2.7 Code",
    hint: "Fast agentic coding",
    priceTier: 2,
    reasoningEfforts: [],
    defaultReasoningEffort: "",
  },
  {
    id: "clark-code:grok45",
    label: "Grok 4.5",
    hint: "Frontier coding · 500K context",
    priceTier: 4,
    reasoningEfforts: ["high", "medium", "low"],
    defaultReasoningEffort: "high",
  },
  {
    id: "clark-code:deepseek_v4_pro",
    label: "DeepSeek V4 Pro",
    hint: "Long-horizon coding · 1M context",
    priceTier: 1,
    reasoningEfforts: ["", "xhigh", "high"],
    defaultReasoningEffort: "",
  },
  {
    id: "clark-code:claude_opus_5",
    label: "Claude Opus 5",
    hint: "Frontier coding · vision · 1M context",
    priceTier: 5,
    reasoningEfforts: [],
    defaultReasoningEffort: "",
  },
  {
    id: "clark-code:gpt56_sol",
    label: "GPT-5.6 Sol",
    hint: "Frontier coding · vision · 1M context",
    priceTier: 5,
    reasoningEfforts: [],
    defaultReasoningEffort: "",
  },
] as const satisfies readonly {
  id: string;
  label: string;
  hint: string;
  priceTier: 1 | 2 | 3 | 4 | 5;
  reasoningEfforts: readonly ReasoningEffortId[];
  defaultReasoningEffort: ReasoningEffortId;
}[];

/** Keep local storage, conversation overrides, and direct callers inside the
 * current picker catalog. Retired choices must not silently reach the provider. */
export function normalizeCodingModel(model: string): string {
  return CODING_MODELS.some((candidate) => candidate.id === model)
    ? model
    : DEFAULT_LOCAL_SETTINGS.model;
}

/** Reasoning choices for one model, in OpenRouter's advertised order. */
export function reasoningEffortsForModel(model: string) {
  const ids = CODING_MODELS.find((candidate) => candidate.id === model)?.reasoningEfforts ?? [""];
  return ids.map((id) => REASONING_EFFORTS.find((effort) => effort.id === id)!);
}

/** Keep persisted and programmatic settings inside the selected model's
 *  OpenRouter contract. Model switches fall back to that model's default. */
export function normalizeReasoningEffort(model: string, effort: string): ReasoningEffortId {
  const config = CODING_MODELS.find((candidate) => candidate.id === model);
  if (!config) {
    return REASONING_EFFORTS.some((candidate) => candidate.id === effort)
      ? (effort as ReasoningEffortId)
      : "";
  }
  return (config.reasoningEfforts as readonly string[]).includes(effort)
    ? (effort as ReasoningEffortId)
    : config.defaultReasoningEffort;
}

/** Short display label for the current model id. */
export function modelLabel(id: string): string {
  const model = normalizeCodingModel(id);
  return CODING_MODELS.find((candidate) => candidate.id === model)!.label;
}

export function loadLocalSettings(): LocalAgentSettings {
  try {
    const raw = localStorage.getItem(KEY);
    const devCwd = import.meta.env.DEV ? env.VITE_CLARK_DEV_CWD?.trim() || "" : "";
    const merged = raw
      ? {
          ...DEFAULT_LOCAL_SETTINGS,
          ...(JSON.parse(raw) as Partial<LocalAgentSettings>),
          ...(devCwd ? { cwd: devCwd } : {}),
        }
      : { ...DEFAULT_LOCAL_SETTINGS, cwd: devCwd };
    const migrated = migrate(merged);
    // Rewrite legacy objects once so obsolete endpoint fields disappear from
    // storage itself, not only from the in-memory settings object.
    if (raw) localStorage.setItem(KEY, JSON.stringify(migrated));
    return migrated;
  } catch {
    return { ...DEFAULT_LOCAL_SETTINGS };
  }
}

// Older installs saved a raw OpenRouter model id (e.g. "z-ai/glm-5.2"). The
// production Clark API only accepts the current Clark tier catalog, so coerce
// stale or retired values to the coding default — otherwise a saved selection
// can reach the provider as an unknown tier. Same for a stale reasoning effort
// the selected model does not support (e.g. "low"/"medium" from an early build)
// → that model's default.
function migrate(s: LocalAgentSettings): LocalAgentSettings {
  const savedModel = typeof s.model === "string" ? s.model : DEFAULT_LOCAL_SETTINGS.model;
  const savedEffort = typeof s.reasoningEffort === "string" ? s.reasoningEffort : "";
  const model = normalizeCodingModel(savedModel);
  // Return the current schema explicitly. Older builds persisted `baseUrl`
  // (often OpenRouter) and spreading the parsed object kept that misleading,
  // unused field alive forever even though Clark Code always uses Clark's API.
  return {
    cwd: typeof s.cwd === "string" ? s.cwd : "",
    model,
    reasoningEffort: normalizeReasoningEffort(model, savedEffort),
    apiKey: typeof s.apiKey === "string" ? s.apiKey : "",
    apiKeyOwner: typeof s.apiKeyOwner === "string" ? s.apiKeyOwner : "",
    computerUseEnabled: s.computerUseEnabled === true,
  };
}

export function saveLocalSettings(settings: LocalAgentSettings): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(migrate(settings)));
  } catch {
    // Non-fatal: settings just won't persist across restarts.
  }
}

// The model + reasoning effort are per-conversation, so each chat can run a
// different model. The global `LocalAgentSettings.model` stays the default a
// new chat seeds from (and the start-screen picker edits); every conversation
// snapshots those values when it is created or first reopened, then updates
// only its own entry. The cloud stores transcripts, not model preferences, so
// these values live in localStorage (scoped by conversation id).
const CHAT_MODELS_KEY = "clark-desktop:chat-models";

export interface ChatModelOverride {
  model: string;
  reasoningEffort: string;
}

function normalizeChatModelOverride(value: unknown): ChatModelOverride {
  const candidate = value && typeof value === "object"
    ? value as Partial<ChatModelOverride>
    : {};
  const model = normalizeCodingModel(
    typeof candidate.model === "string" ? candidate.model : DEFAULT_LOCAL_SETTINGS.model,
  );
  return {
    model,
    reasoningEffort: normalizeReasoningEffort(
      model,
      typeof candidate.reasoningEffort === "string" ? candidate.reasoningEffort : "",
    ),
  };
}

function normalizeChatModels(models: Record<string, unknown>): Record<string, ChatModelOverride> {
  return Object.fromEntries(
    Object.entries(models).map(([id, value]) => [id, normalizeChatModelOverride(value)]),
  );
}

export function loadChatModels(): Record<string, ChatModelOverride> {
  try {
    const raw = localStorage.getItem(CHAT_MODELS_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    const models = normalizeChatModels(parsed as Record<string, unknown>);
    if (JSON.stringify(models) !== raw) localStorage.setItem(CHAT_MODELS_KEY, JSON.stringify(models));
    return models;
  } catch {
    return {};
  }
}

export function saveChatModels(models: Record<string, ChatModelOverride>): void {
  try {
    localStorage.setItem(CHAT_MODELS_KEY, JSON.stringify(normalizeChatModels(models)));
  } catch {
    // Non-fatal.
  }
}

/** The model + reasoning effort the ACTIVE conversation runs with. The global
 *  default is only a fallback for the start screen and legacy chats that have
 *  not yet been reopened and pinned on this device. */
export function effectiveModelSettings(
  base: LocalAgentSettings,
  chatModels: Record<string, ChatModelOverride>,
  chatId: string | null,
): LocalAgentSettings {
  const baseModel = normalizeCodingModel(base.model);
  if (!chatId) {
    return {
      ...base,
      model: baseModel,
      reasoningEffort: normalizeReasoningEffort(baseModel, base.reasoningEffort),
    };
  }
  const ov = chatModels[chatId];
  if (!ov) {
    return {
      ...base,
      model: baseModel,
      reasoningEffort: normalizeReasoningEffort(baseModel, base.reasoningEffort),
    };
  }
  const model = normalizeCodingModel(ov.model || baseModel);
  const reasoningEffort = ov.reasoningEffort !== undefined
    ? ov.reasoningEffort
    : base.reasoningEffort;
  return {
    ...base,
    model,
    reasoningEffort: normalizeReasoningEffort(model, reasoningEffort),
  };
}

// Durable memory is a single global (per-user) preference, on by default. When
// on, the agent gets the `memory` tool and its saved facts (project + global)
// are injected into the system prompt.
const MEMORIES_KEY = "clark-desktop:memories-enabled";

export function loadMemoriesEnabled(): boolean {
  try {
    return localStorage.getItem(MEMORIES_KEY) !== "false";
  } catch {
    return true;
  }
}

export function saveMemoriesEnabled(on: boolean): void {
  try {
    localStorage.setItem(MEMORIES_KEY, String(on));
  } catch {
    // Non-fatal.
  }
}

// Experimental `browser` tool (clark-browser, downloaded on first use). Off
// by default — Alpha-status, ~150-300MB, not bundled in the app.
const BROWSER_KEY = "clark-desktop:browser-enabled";

export function loadBrowserEnabled(): boolean {
  try {
    return localStorage.getItem(BROWSER_KEY) === "true";
  } catch {
    return false;
  }
}

export function saveBrowserEnabled(on: boolean): void {
  try {
    localStorage.setItem(BROWSER_KEY, String(on));
  } catch {
    // Non-fatal.
  }
}

// Bounded multi-agent fan-out is available by default for local projects. The
// model-facing policy remains explicit-request-only; coding writers use
// isolated clones and need a separate apply step.
const ORCHESTRATION_KEY = "clark-desktop:orchestration-enabled";

export function loadOrchestrationEnabled(): boolean {
  try {
    return localStorage.getItem(ORCHESTRATION_KEY) !== "false";
  } catch {
    return true;
  }
}

export function saveOrchestrationEnabled(on: boolean): void {
  try {
    localStorage.setItem(ORCHESTRATION_KEY, String(on));
  } catch {
    // Non-fatal.
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
  const model = normalizeCodingModel(s.model);
  const reasoningEffort = normalizeReasoningEffort(model, s.reasoningEffort);
  return {
    cwd: remote ? undefined : project || undefined,
    auth_token: s.apiKey.trim() || undefined,
    extra: {
      model,
      // "" = let the model's server-side default apply.
      ...(reasoningEffort ? { reasoning_effort: reasoningEffort } : {}),
      // Per-project shell-command policy the engine consults to skip / block the gate.
      command_allowlist: loadAllowlist(project),
      command_denylist: loadDenylist(project),
      // MCP servers to spawn + expose as tools.
      mcp_servers: enabledMcpConfigs(loadMcpServers()),
      // Durable memory: exposes the `memory` tool + injects saved facts.
      memories: loadMemoriesEnabled(),
      // Private Git evidence sync and per-turn repository context recall.
      project_knowledge: projectKnowledgeEnabled(),
      // Experimental `browser` tool — off unless the user opted in.
      browser_enabled: loadBrowserEnabled(),
      // Native computer use — independently fail-closed by the host, signed
      // helper, OS privacy grants, per-app approvals, and action policy.
      computer_use_enabled: s.computerUseEnabled === true,
      // Conservative-by-default parallel investigation and isolated coding
      // workstreams. Remote hosts need a proven isolation boundary first.
      orchestration: { enabled: !remote && loadOrchestrationEnabled() },
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

/** Forget one folder from the project list without touching its files or chats. */
export function removeRecentProject(path: string): string[] {
  const next = loadRecentProjects().filter((candidate) => candidate !== path.trim());
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
