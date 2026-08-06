import { projectClarkCodeBilling, type BillingSummary } from "./billing";
import type { ConnectConfig } from "../core-bridge/bridge";
import type { RemoteInfo } from "./remoteWorker";
import type { CloudAdvisorTarget, ScoutCartographyTarget } from "./localAgent";
import firstPartyCatalog from "./first-party-specialists.json";

export type SpecialistKind = string;
export type SpecialistWorkflow = string;
export type SpecialistTab = string;
export type ScoutTab = "map" | "changes" | "simulations" | "evidence" | "runs";
export type SecurityTab = "posture" | "findings" | "zero-days" | "campaigns" | "scans";
export type ScientistTab = "programs" | "campaigns" | "experiments" | "evidence" | "runs";
export type RsiTab = "worlds" | "evaluations" | "runs" | "frontier" | "evidence";

export interface SpecialistContext {
  kind: SpecialistKind;
  organizationId?: string;
  workspaceId?: string;
  repositoryId?: string;
  objectKind?: string;
  objectId?: string;
  workflow?: SpecialistWorkflow;
  programId?: string;
  campaignId?: string;
  studyId?: string;
  experimentId?: string;
  runId?: string;
  targetId?: string;
}

export interface RsiScoutContextSnapshot {
  schemaVersion: 1;
  workspaceId: string;
  entries: Array<{
    objectKind: string;
    objectId: string;
    classification: string;
    attributes: Record<string, unknown>;
  }>;
}

interface SkillIdentity {
  id: string;
  revision: string;
  invocationName: string;
  enabled: boolean;
}

export interface SpecialistSkillReference {
  type: "skill_reference";
  id: string;
  revision: string;
  name: string;
}

export interface SpecialistDefinition {
  kind: SpecialistKind;
  version: string;
  label: string;
  headline: string;
  value: string;
  engine: "skill" | "research_runtime";
  tabs: ReadonlyArray<{ id: SpecialistTab; label: string }>;
  defaultTab: SpecialistTab;
  defaultWorkflow: SpecialistWorkflow;
  skillBindings: Readonly<Record<SpecialistWorkflow, string>>;
  slashCommands: ReadonlyArray<{
    prefixes: readonly string[];
    tab: SpecialistTab;
    workflow: SpecialistWorkflow;
    promptPrefix?: string;
  }>;
}

export interface SpecialistRegistry {
  readonly ordered: readonly SpecialistDefinition[];
  readonly byKind: Readonly<Record<string, SpecialistDefinition>>;
  get(kind: string): SpecialistDefinition | undefined;
}

export interface SpecialistCatalogReceipt {
  schemaVersion: number;
  catalogVersion: string;
  catalogSha256: string;
  trust: {
    source: "signed_app_bundle";
    requiresSignedReleaseBinary: boolean;
  };
  manifests: SpecialistDefinition[];
}

export function createSpecialistRegistry(
  definitions: readonly SpecialistDefinition[],
): SpecialistRegistry {
  const byKind: Record<string, SpecialistDefinition> = {};
  for (const definition of definitions) {
    if (
      !/^[a-z][a-z0-9_-]{1,63}$/.test(definition.kind)
      || byKind[definition.kind]
      || !definition.tabs.some((tab) => tab.id === definition.defaultTab)
      || !definition.slashCommands.every((command) =>
        definition.tabs.some((tab) => tab.id === command.tab))
    ) {
      throw new Error(`Invalid or duplicate specialist manifest: ${definition.kind}`);
    }
    byKind[definition.kind] = Object.freeze(definition);
  }
  const ordered = Object.freeze([...definitions]);
  return Object.freeze({
    ordered,
    byKind: Object.freeze(byKind),
    get: (kind: string) => byKind[kind],
  });
}

function catalogObject(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} is not an object`);
  }
  return value as Record<string, unknown>;
}

function catalogString(value: unknown, label: string): string {
  if (typeof value !== "string" || !value) throw new Error(`${label} is invalid`);
  return value;
}

export function parseSpecialistCatalog(value: unknown): SpecialistCatalogReceipt {
  const root = catalogObject(value, "Specialist catalog");
  const trust = catalogObject(root.trust, "Specialist catalog trust");
  if (
    root.schemaVersion !== 1
    || root.catalogVersion !== "1.0.0"
    || typeof root.catalogSha256 !== "string"
    || !/^[a-f0-9]{64}$/.test(root.catalogSha256)
    || trust.source !== "signed_app_bundle"
    || trust.requiresSignedReleaseBinary !== true
    || !Array.isArray(root.manifests)
  ) {
    throw new Error("Specialist catalog does not match the signed v1 contract");
  }
  const manifests = root.manifests.map((entry, index): SpecialistDefinition => {
    const manifest = catalogObject(entry, `Specialist manifest ${index}`);
    const engine = manifest.engine;
    if (engine !== "skill" && engine !== "research_runtime") {
      throw new Error(`Specialist manifest ${index} has an invalid engine`);
    }
    if (!Array.isArray(manifest.tabs) || !Array.isArray(manifest.slashCommands)) {
      throw new Error(`Specialist manifest ${index} has invalid navigation`);
    }
    const skillBindings = catalogObject(
      manifest.skillBindings,
      `Specialist manifest ${index} skill bindings`,
    );
    if (Object.values(skillBindings).some((binding) => typeof binding !== "string")) {
      throw new Error(`Specialist manifest ${index} has an invalid skill binding`);
    }
    return {
      kind: catalogString(manifest.kind, "specialist kind"),
      version: catalogString(manifest.version, "specialist version"),
      label: catalogString(manifest.label, "specialist label"),
      headline: catalogString(manifest.headline, "specialist headline"),
      value: catalogString(manifest.value, "specialist value"),
      engine,
      defaultTab: catalogString(manifest.defaultTab, "specialist default tab"),
      defaultWorkflow: catalogString(
        manifest.defaultWorkflow,
        "specialist default workflow",
      ),
      skillBindings: skillBindings as Readonly<Record<SpecialistWorkflow, string>>,
      tabs: manifest.tabs.map((tab, tabIndex) => {
        const row = catalogObject(tab, `Specialist tab ${tabIndex}`);
        return {
          id: catalogString(row.id, "specialist tab id"),
          label: catalogString(row.label, "specialist tab label"),
        };
      }),
      slashCommands: manifest.slashCommands.map((command, commandIndex) => {
        const row = catalogObject(command, `Specialist command ${commandIndex}`);
        if (!Array.isArray(row.prefixes)) {
          throw new Error(`Specialist command ${commandIndex} has invalid prefixes`);
        }
        const prefixes = row.prefixes.map((prefix) =>
          catalogString(prefix, "specialist command prefix"));
        return {
          prefixes,
          tab: catalogString(row.tab, "specialist command tab"),
          workflow: catalogString(row.workflow, "specialist command workflow"),
          ...(row.promptPrefix === undefined
            ? {}
            : { promptPrefix: catalogString(row.promptPrefix, "specialist prompt prefix") }),
        };
      }),
    };
  });
  return {
    schemaVersion: 1,
    catalogVersion: "1.0.0",
    catalogSha256: root.catalogSha256,
    trust: {
      source: "signed_app_bundle",
      requiresSignedReleaseBinary: true,
    },
    manifests,
  };
}

export const FIRST_PARTY_SPECIALIST_CATALOG = parseSpecialistCatalog(firstPartyCatalog);
export const SPECIALIST_CATALOG_SHA256 =
  FIRST_PARTY_SPECIALIST_CATALOG.catalogSha256;

export const SPECIALIST_REGISTRY = createSpecialistRegistry(
  FIRST_PARTY_SPECIALIST_CATALOG.manifests,
);
export const SPECIALISTS = SPECIALIST_REGISTRY.byKind;
export const SPECIALIST_KINDS = SPECIALIST_REGISTRY.ordered.map(({ kind }) => kind);

export function researchRuntimeSpecialist(
  context: SpecialistContext | null | undefined,
): SpecialistDefinition | null {
  if (!context) return null;
  const definition = SPECIALIST_REGISTRY.get(context.kind);
  return definition?.engine === "research_runtime" ? definition : null;
}

export function skillAdvisorTarget(
  context: SpecialistContext | null | undefined,
  trainingEnabled = false,
): CloudAdvisorTarget | undefined {
  if (
    !context?.organizationId?.trim()
    || (context.kind !== "scout" && context.kind !== "security")
  ) return undefined;
  const definition = SPECIALIST_REGISTRY.get(context.kind);
  if (!definition || definition.engine !== "skill") return undefined;
  return {
    organizationId: context.organizationId,
    specialist: context.kind,
    workflow: context.workflow || definition.defaultWorkflow,
    ...(trainingEnabled ? { trainingConsent: "explicit_user" as const } : {}),
  };
}

/** Bind the Scout run to the workspace selected by the first-party canvas.
 * The native host adds the private identity root; the model never receives
 * this binding as an argument. */
export function scoutCartographyTarget(
  context: SpecialistContext | null | undefined,
  remote?: Pick<RemoteInfo, "arch"> | null,
  targetId?: string | null,
): ScoutCartographyTarget | undefined {
  if (
    context?.kind !== "scout"
    || !context.organizationId?.trim()
    || !context.workspaceId?.trim()
  ) return undefined;
  const architecture = remote?.arch?.trim();
  const separator = architecture?.indexOf("-") ?? -1;
  return {
    organizationId: context.organizationId,
    workspaceId: context.workspaceId,
    ...(separator > 0
      ? {
        platform: architecture!.slice(0, separator),
        architecture: architecture!.slice(separator + 1),
      }
      : {}),
    ...(targetId?.trim() ? { targetId: targetId.trim() } : {}),
  };
}

/** Build the WebView-owned portion of the internal provider configuration.
 * Native code replaces the executable and runtime paths before spawning. */
export function specialistConnectConfig(
  context: SpecialistContext,
  cwd: string,
  scoutContext?: RsiScoutContextSnapshot,
  remote?: { host: string; remoteRoot: string },
  advisorTrainingEnabled = false,
): ConnectConfig {
  const definition = researchRuntimeSpecialist(context);
  const project = cwd.trim();
  if (!definition || !project) {
    throw new Error("A registered research specialist and local project folder are required.");
  }
  const workflow = context.workflow || definition.defaultWorkflow;
  const validWorkflow = workflow === definition.defaultWorkflow
    || definition.slashCommands.some((command) => command.workflow === workflow);
  if (!validWorkflow) {
    throw new Error(`Unsupported ${definition.label} workflow: ${workflow}`);
  }
  return {
    cwd: project,
    extra: {
      specialist: definition.kind,
      workflow,
      ...(context.organizationId ? { organizationId: context.organizationId } : {}),
      ...(context.workspaceId || scoutContext?.workspaceId
        ? { workspaceId: context.workspaceId || scoutContext!.workspaceId }
        : {}),
      ...(definition.kind === "rsi" && scoutContext
        ? { scoutContext }
        : {}),
      modelRoute: "clark_deepseek_v4_latest",
      maxIterations: 3,
      ...(advisorTrainingEnabled ? { advisorTrainingEnabled: true } : {}),
      ...(remote ? { remote } : {}),
    },
  };
}

export function isSpecialistKind(value: string): value is SpecialistKind {
  return Boolean(SPECIALIST_REGISTRY.get(value));
}

export type SpecialistAccessState =
  | "loading"
  | "signed_out"
  | "free"
  | "ready"
  | "action_needed"
  | "scope_lost"
  | "offline";

export function specialistAccessBadge(state: SpecialistAccessState): string {
  switch (state) {
    case "ready":
      return "Access ready";
    case "free":
      return "Not included";
    case "loading":
      return "Checking access";
    case "signed_out":
      return "Sign in";
    case "action_needed":
      return "Action needed";
    case "scope_lost":
      return "Access changed";
    case "offline":
      return "Can't verify access";
  }
}

/** Local projection used while the organization-bound server entitlement is
 * loading. Shared billing policy supplies coverage and usage admission; the
 * entitlement API remains authoritative before specialist data or actions. */
export function projectedSpecialistAccess(
  signedIn: boolean,
  billing: BillingSummary | null,
): SpecialistAccessState {
  if (!signedIn) return "signed_out";
  if (!billing) return "loading";
  switch (projectClarkCodeBilling(billing).coverage.state) {
    case "ready": return "ready";
    case "action_needed": return "action_needed";
    case "not_included": return "free";
    case "unknown": return "loading";
  }
}

/** Keep an already verified entitlement distinct from a later specialist-data
 * failure. A failed overview/query is a canvas error, not evidence that the
 * user's paid coverage became unverifiable. */
export function specialistAccessAfterLoadFailure(
  entitlementVerified: boolean,
): Extract<SpecialistAccessState, "ready" | "offline"> {
  return entitlementVerified ? "ready" : "offline";
}

export function specialistAccessCopy(
  state: SpecialistAccessState,
  kind: SpecialistKind,
): { title: string; detail: string; action: "sign_in" | "upgrade" | "billing" | "retry" | null } {
  const definition = SPECIALIST_REGISTRY.get(kind);
  if (!definition) {
    return {
      title: "Unknown specialist",
      detail: "This specialist is not registered in this Clark build.",
      action: null,
    };
  }
  const label = definition.label;
  switch (state) {
    case "loading":
      return { title: `Checking ${label} access…`, detail: "This usually takes a moment.", action: null };
    case "signed_out":
      return {
        title: `Sign in to use Clark ${label}`,
        detail: `${definition.value} Specialist work is saved to your Clark account.`,
        action: "sign_in",
      };
    case "action_needed":
      return {
        title: `${label} is paused`,
        detail: "Update billing or ask a workspace admin to restore your paid seat. Saved work is safe.",
        action: "billing",
      };
    case "scope_lost":
      return {
        title: `${label} workspace access changed`,
        detail: "This conversation belongs to a workspace you can no longer access. Saved work remains protected.",
        action: null,
      };
    case "offline":
      return {
        title: `${label} could not verify access`,
        detail: "Reconnect to check your current coverage. Saved views remain available after access returns.",
        action: "retry",
      };
    case "free":
      return {
        title: `Unlock Clark ${label}`,
        detail: `Clark ${label} is available with Pro coverage. ${definition.value} Your existing chats and Clark Code remain available.`,
        action: "upgrade",
      };
    case "ready":
      return { title: `${label} is ready`, detail: definition.value, action: null };
  }
}

export function isSpecialistTab(kind: SpecialistKind, value: string): value is SpecialistTab {
  return SPECIALIST_REGISTRY.get(kind)?.tabs.some((tab) => tab.id === value) ?? false;
}

export function specialistDeepLink(search: string): {
  kind: SpecialistKind;
  tab?: SpecialistTab;
} | null {
  const params = new URLSearchParams(search);
  const kind = params.get("specialist");
  if (!kind || !isSpecialistKind(kind)) return null;
  const tab = params.get("tab");
  return {
    kind,
    ...(tab && isSpecialistTab(kind, tab) ? { tab } : {}),
  };
}

export function specialistSlashIntent(text: string): {
  kind: SpecialistKind;
  tab: SpecialistTab;
  prompt: string;
  workflow: SpecialistWorkflow;
} | null {
  const value = text.trimStart();
  const commands = SPECIALIST_REGISTRY.ordered
    .flatMap((definition) => definition.slashCommands.map((command) => ({
      ...command,
      kind: definition.kind,
    })))
    .sort((left, right) =>
      Math.max(...right.prefixes.map((prefix) => prefix.length))
      - Math.max(...left.prefixes.map((prefix) => prefix.length)));
  for (const command of commands) {
    const prefix = command.prefixes.find((candidate) => value.startsWith(candidate));
    if (!prefix) continue;
    const remainder = value.slice(prefix.length);
    if (remainder && !/^\s/.test(remainder)) continue;
    return {
      kind: command.kind,
      tab: command.tab,
      prompt: `${command.promptPrefix ?? ""}${remainder.trim()}`.trim(),
      workflow: command.workflow,
    };
  }
  return null;
}

/** Specialist spaces activate their engine contract without exposing a
 * `$plugin:skill` token in the user's prose. The reference is still revision
 * pinned exactly like a skill selected in the ordinary composer. */
export function withActiveSpecialistSkill(
  selected: SpecialistSkillReference[],
  catalog: readonly SkillIdentity[],
  active: SpecialistKind | null,
  workflow?: SpecialistWorkflow,
): SpecialistSkillReference[] {
  if (!active) return selected;
  const definition = SPECIALIST_REGISTRY.get(active);
  if (!definition) return selected;
  const invocationName = definition.skillBindings[
    workflow ?? definition.defaultWorkflow
  ];
  if (!invocationName) return selected;
  if (selected.some((reference) => reference.name === invocationName)) return selected;
  const skill = catalog.find(
    (candidate) => candidate.enabled && candidate.invocationName === invocationName,
  );
  return skill
    ? [...selected, {
        type: "skill_reference",
        id: skill.id,
        revision: skill.revision,
        name: skill.invocationName,
      }]
    : selected;
}

export function specialistWorkflowAvailable(
  references: readonly SpecialistSkillReference[],
  active: SpecialistKind | null,
  workflow?: SpecialistWorkflow,
): boolean {
  if (!active) return true;
  const definition = SPECIALIST_REGISTRY.get(active);
  if (!definition) return false;
  const invocationName = definition.skillBindings[
    workflow ?? definition.defaultWorkflow
  ];
  return !invocationName || references.some(
    (reference) => reference.name === invocationName,
  );
}
