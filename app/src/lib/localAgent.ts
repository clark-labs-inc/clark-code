// Settings for the local coding provider. The open foundation owns the local
// loop; the active product supplies its selectable models and managed routes.
//
// Non-secret preferences are persisted in account-scoped localStorage. Model
// and integration credentials are resolved only by the native encrypted store.

import type { ConnectConfig } from "../core-bridge/bridge";
import { loadAllowlist, loadDenylist } from "./commandPolicy";
import { loadMcpServers, enabledMcpConfigs } from "./mcpServers";
import type { RemoteTargetConfig } from "./remoteWorker";
import { projectKnowledgeEnabled } from "./repositoryKnowledge";
import { productModule } from "../product/productModule";
import {
  accountScopedKey,
  loadProjectCwd,
  normalizedAccountScope,
  saveProjectCwd,
} from "./accountProjectStorage";

export {
  addRecentProject,
  loadRecentProjects,
  removeRecentProject,
} from "./accountProjectStorage";

export interface ScoutCartographyTarget {
  organizationId: string;
  workspaceId: string;
  platform?: string;
  architecture?: string;
  targetId?: string;
  runRequestId?: string;
}

export interface ProductSpecialistTarget {
  organizationId: string;
  kind: string;
  workflow: string;
  trainingOptIn?: boolean;
}

const KEY = "agent-desktop:local-agent";
const env = import.meta.env as Record<string, string | undefined>;

export interface LocalAgentSettings {
  /** Absolute path to the project the agent edits. */
  cwd: string;
  /** Product-advertised model id. */
  model: string;
  /** Model-specific maximum reasoning effort sent with each request. */
  reasoningEffort: string;
  /** Opt-in local control of ordinary desktop applications. Native support is
   *  platform-gated again by the host and by OS privacy permissions. */
  computerUseEnabled?: boolean;
  /** Explicit opt-in for using bounded advisor packets and outcome feedback
   * to improve future product advisors. Operational telemetry is still retained
   * when off, but remains training-ineligible. */
  advisorTrainingEnabled?: boolean;
}

const productModels = productModule().localAgent;

export const DEFAULT_LOCAL_SETTINGS: LocalAgentSettings = {
  cwd: "",
  model: productModels.defaultModel,
  reasoningEffort: productModels.defaultReasoningEffort,
  computerUseEnabled: false,
  advisorTrainingEnabled: false,
};

/** Reasoning effort ids accepted by the active product's models. */
export type ReasoningEffortId =
  | ""
  | "max"
  | "xhigh"
  | "high"
  | "medium"
  | "low"
  | "minimal";

/** Specialist sessions are a controlled product lane, not a user-selectable
 * coding session. Their model and reasoning contract is pinned end to end. */
export const SPECIALIST_MODEL_ID = productModels.specialistModel?.id ?? productModels.defaultModel;
export const SPECIALIST_MODEL_LABEL = productModels.specialistModel?.label
  ?? productModels.models.find((model) => model.id === productModels.defaultModel)?.label
  ?? "Product model";
export const SPECIALIST_REASONING_EFFORT: ReasoningEffortId =
  productModels.specialistModel?.defaultReasoningEffort ?? productModels.defaultReasoningEffort;

/** The single choice included with the product's default usage policy. */
export const INCLUDED_CODING_MODEL_ID = productModels.includedModel ?? "";

/** The coding models the active product exposes in the composer picker.
 *  `defaultReasoningEffort` is the highest effort supported by that model. */
export const CODING_MODELS = productModels.models;

/** Keep included-usage UI gated on the selected managed lane, never on a raw model
 * name that happens to resolve to the same upstream provider. */
export function isIncludedCodingModel(model: string): boolean {
  return INCLUDED_CODING_MODEL_ID.length > 0 && model === INCLUDED_CODING_MODEL_ID;
}

/** Keep local storage, conversation overrides, and direct callers inside the
 * current picker catalog. Retired choices must not silently reach the provider. */
export function normalizeCodingModel(model: string): string {
  return (productModels.specialistModel !== undefined && model === SPECIALIST_MODEL_ID)
    || CODING_MODELS.some((candidate) => candidate.id === model)
    ? model
    : DEFAULT_LOCAL_SETTINGS.model;
}

/** Keep persisted and programmatic settings at the selected model's maximum
 *  supported OpenRouter effort. User-selectable effort overrides are retired. */
export function normalizeReasoningEffort(model: string, _effort: string): ReasoningEffortId {
  if (productModels.specialistModel !== undefined && model === SPECIALIST_MODEL_ID) {
    return SPECIALIST_REASONING_EFFORT;
  }
  const config = CODING_MODELS.find((candidate) => candidate.id === model);
  return config?.defaultReasoningEffort ?? "xhigh";
}

/** Short display label for the current model id. */
export function modelLabel(id: string): string {
  const model = normalizeCodingModel(id);
  if (productModels.specialistModel !== undefined && model === SPECIALIST_MODEL_ID) {
    return SPECIALIST_MODEL_LABEL;
  }
  return CODING_MODELS.find((candidate) => candidate.id === model)!.label;
}

export function loadLocalSettings(scope?: string | null): LocalAgentSettings {
  try {
    const accountScope = normalizedAccountScope(scope);
    const scopedKey = accountScopedKey(KEY, accountScope);
    const scopedRaw = localStorage.getItem(scopedKey);
    const raw = scopedRaw;
    const devCwd = import.meta.env.DEV ? env.VITE_AGENT_DESKTOP_DEV_CWD?.trim() || "" : "";
    const merged = raw
      ? {
          ...DEFAULT_LOCAL_SETTINGS,
          ...(JSON.parse(raw) as Partial<LocalAgentSettings>),
          ...(devCwd ? { cwd: devCwd } : {}),
        }
      : { ...DEFAULT_LOCAL_SETTINGS, cwd: devCwd };
    const normalized = normalizeSettings(merged);
    const cwd = devCwd || loadProjectCwd(accountScope);
    if (raw) {
      localStorage.setItem(
        scopedKey,
        JSON.stringify({ ...normalized, cwd: "" }),
      );
    }
    return { ...normalized, cwd };
  } catch {
    return { ...DEFAULT_LOCAL_SETTINGS };
  }
}

function normalizeSettings(s: LocalAgentSettings): LocalAgentSettings {
  const savedModel = typeof s.model === "string" ? s.model : DEFAULT_LOCAL_SETTINGS.model;
  const savedEffort = typeof s.reasoningEffort === "string" ? s.reasoningEffort : "";
  const model = normalizeCodingModel(savedModel);
  const reasoningEffort = model === savedModel
    ? normalizeReasoningEffort(model, savedEffort)
    : DEFAULT_LOCAL_SETTINGS.reasoningEffort;
  // Reconstruct the closed current schema instead of retaining unknown fields.
  return {
    cwd: typeof s.cwd === "string" ? s.cwd : "",
    model,
    reasoningEffort,
    computerUseEnabled: s.computerUseEnabled === true,
    advisorTrainingEnabled: s.advisorTrainingEnabled === true,
  };
}

export function saveLocalSettings(settings: LocalAgentSettings, scope?: string | null): void {
  try {
    const normalized = normalizeSettings(settings);
    const accountScope = normalizedAccountScope(scope);
    saveProjectCwd(accountScope, normalized.cwd);
    localStorage.setItem(
      accountScopedKey(KEY, accountScope),
      JSON.stringify({ ...normalized, cwd: "" }),
    );
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
const CHAT_MODELS_KEY = "agent-desktop:chat-models";

export interface ChatModelOverride {
  model: string;
  reasoningEffort: string;
}

function normalizeChatModelOverride(value: unknown): ChatModelOverride {
  const candidate = value && typeof value === "object"
    ? value as Partial<ChatModelOverride>
    : {};
  const savedModel = typeof candidate.model === "string"
    ? candidate.model
    : DEFAULT_LOCAL_SETTINGS.model;
  const model = normalizeCodingModel(savedModel);
  return {
    model,
    reasoningEffort: model === savedModel
      ? normalizeReasoningEffort(
        model,
        typeof candidate.reasoningEffort === "string" ? candidate.reasoningEffort : "",
      )
      : DEFAULT_LOCAL_SETTINGS.reasoningEffort,
  };
}

function normalizeChatModels(models: Record<string, unknown>): Record<string, ChatModelOverride> {
  return Object.fromEntries(
    Object.entries(models).map(([id, value]) => [id, normalizeChatModelOverride(value)]),
  );
}

export function loadChatModels(scope?: string | null): Record<string, ChatModelOverride> {
  try {
    const key = accountScopedKey(CHAT_MODELS_KEY, scope);
    const raw = localStorage.getItem(key);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    const models = normalizeChatModels(parsed as Record<string, unknown>);
    if (JSON.stringify(models) !== raw) localStorage.setItem(key, JSON.stringify(models));
    return models;
  } catch {
    return {};
  }
}

export function saveChatModels(
  models: Record<string, ChatModelOverride>,
  scope?: string | null,
): void {
  try {
    localStorage.setItem(
      accountScopedKey(CHAT_MODELS_KEY, scope),
      JSON.stringify(normalizeChatModels(models)),
    );
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
const MEMORIES_KEY = "agent-desktop:memories-enabled";

export function loadMemoriesEnabled(scope?: string | null): boolean {
  try {
    return localStorage.getItem(accountScopedKey(MEMORIES_KEY, scope)) !== "false";
  } catch {
    return true;
  }
}

export function saveMemoriesEnabled(on: boolean, scope?: string | null): void {
  try {
    localStorage.setItem(accountScopedKey(MEMORIES_KEY, scope), String(on));
  } catch {
    // Non-fatal.
  }
}

// Experimental `browser` tool (managed browser, downloaded on first use). Off
// by default — Alpha-status, ~150-300MB, not bundled in the app.
const BROWSER_KEY = "agent-desktop:browser-enabled";

export function loadBrowserEnabled(scope?: string | null): boolean {
  try {
    return localStorage.getItem(accountScopedKey(BROWSER_KEY, scope)) === "true";
  } catch {
    return false;
  }
}

export function saveBrowserEnabled(on: boolean, scope?: string | null): void {
  try {
    localStorage.setItem(accountScopedKey(BROWSER_KEY, scope), String(on));
  } catch {
    // Non-fatal.
  }
}

// Bounded multi-agent fan-out is available by default for local projects. The
// model-facing policy remains explicit-request-only; coding writers use
// isolated clones and need a separate apply step.
const ORCHESTRATION_KEY = "agent-desktop:orchestration-enabled";

export function loadOrchestrationEnabled(scope?: string | null): boolean {
  try {
    return localStorage.getItem(accountScopedKey(ORCHESTRATION_KEY, scope)) !== "false";
  } catch {
    return true;
  }
}

export function saveOrchestrationEnabled(on: boolean, scope?: string | null): void {
  try {
    localStorage.setItem(accountScopedKey(ORCHESTRATION_KEY, scope), String(on));
  } catch {
    // Non-fatal.
  }
}

/**
 * Build the `connect` config the native coding provider expects. Everything
 * routes through the provider selected by the native product composition.
 * Credentials remain in the native encrypted store.
 */
export function localConnectConfig(
  s: LocalAgentSettings,
  remote?: RemoteTargetConfig,
  scout?: ScoutCartographyTarget,
  specialistKind?: string,
  scope?: string | null,
  productSpecialist?: ProductSpecialistTarget,
  specialistModel?: { model: string; reasoningEffort: string },
  sandboxReadRoots: string[] = [],
): ConnectConfig {
  // For a remote project the root lives on the remote host; tool I/O runs there
  // inside the durable worker. The command policy is keyed by the project path.
  const project = (remote ? remote.cwd : s.cwd).trim();
  const isSpecialist = Boolean(specialistKind?.trim());
  const model = specialistModel?.model
    ?? (isSpecialist ? SPECIALIST_MODEL_ID : normalizeCodingModel(s.model));
  const reasoningEffort = specialistModel?.reasoningEffort
    ?? (isSpecialist
      ? SPECIALIST_REASONING_EFFORT
      : normalizeReasoningEffort(model, s.reasoningEffort));
  if (remote) {
    return { extra: { remote_worker: remote } };
  }
  return {
    cwd: project || undefined,
    extra: {
      model,
      reasoning_effort: reasoningEffort,
      // Per-project shell-command policy the engine consults to skip / block the gate.
      command_allowlist: loadAllowlist(project, scope),
      command_denylist: loadDenylist(project, scope),
      // MCP servers to spawn + expose as tools.
      mcp_servers: enabledMcpConfigs(loadMcpServers(scope)),
      // Durable memory: exposes the `memory` tool + injects saved facts.
      memories: loadMemoriesEnabled(scope),
      // Private Git evidence sync and per-turn repository context recall.
      project_knowledge: projectKnowledgeEnabled(scope),
      // Experimental `browser` tool — off unless the user opted in.
      browser_enabled: loadBrowserEnabled(scope),
      // Native computer use — independently fail-closed by the host, signed
      // helper, OS privacy grants, per-app approvals, and action policy.
      computer_use_enabled: s.computerUseEnabled === true,
      // Conservative-by-default parallel investigation and isolated coding
      // workstreams. The engine keeps delegated read-only children on the remote
      // host inside the worker's account/project boundary.
      orchestration: { enabled: loadOrchestrationEnabled(scope) },
      ...(sandboxReadRoots.length > 0
        ? { sandbox_read_roots: [...new Set(sandboxReadRoots.filter((root) => root.trim()))] }
        : {}),
      ...(scout ? {
        scout_cartography: {
          organization_id: scout.organizationId,
          workspace_id: scout.workspaceId,
          ...(scout.runRequestId ? { human_run_request_id: scout.runRequestId } : {}),
          ...(scout.platform ? { platform: scout.platform } : {}),
          ...(scout.architecture ? { architecture: scout.architecture } : {}),
          ...(scout.targetId ? { target_id: scout.targetId } : {}),
        },
      } : {}),
      ...(productModels.providerExtra?.({
        ...(productSpecialist ? {
          specialist: {
            organizationId: productSpecialist.organizationId,
            kind: productSpecialist.kind,
            workflow: productSpecialist.workflow,
          },
        } : {}),
        trainingOptIn: productSpecialist?.trainingOptIn === true,
        executionResidency: "local_only",
      }) ?? {}),
      // Native product composition may use the canonical conversation recipe
      // to expose read-only capabilities, but never as entitlement authority.
      // Keep this after product extras so they cannot replace the active recipe.
      ...(specialistKind?.trim()
        ? { specialist_kind: specialistKind.trim() }
        : {}),
      // When present, the provider runs this session's tools on the remote host.
    },
  };
}

/** Whether the settings are complete enough to start a session. Managed product
 * credentials are provisioned by the native host, so they are not required here. */
export function localSettingsReady(s: LocalAgentSettings): string | null {
  if (!s.cwd.trim()) return "Choose a project folder.";
  return null;
}


/** The last path segment of a folder, for compact display. */
export function projectName(path: string): string {
  const cleaned = path.replace(/[/\\]+$/, "");
  const parts = cleaned.split(/[/\\]/);
  return parts[parts.length - 1] || cleaned || path;
}
