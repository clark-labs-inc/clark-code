import { capabilityAccess, type ProductAccessProjection } from "./productAccess";
import type { ConnectConfig } from "../core-bridge/bridge";
import type { RemoteInfo } from "./remoteWorker";
import type { ProductSpecialistTarget, ScoutCartographyTarget } from "./localAgent";
import { productModule } from "../product/productModule";

// The renderer owns the presentation adapters for every specialist kind that
// a signed product catalog may register. Product-specific policy and runtime
// ownership stay in the downstream catalog; this list only gates whether the
// foundation can safely render the catalog entry.
const SUPPORTED_SPECIALIST_KINDS = ["spec", "scout", "security", "scientist", "rsi"] as const;
const SUPPORTED_SPECIALIST_KIND_SET = new Set<string>(SUPPORTED_SPECIALIST_KINDS);

export type SpecialistKind = typeof SUPPORTED_SPECIALIST_KINDS[number];

export function isSupportedSpecialistKind(value: string): value is SpecialistKind {
  return SUPPORTED_SPECIALIST_KIND_SET.has(value);
}
export type SpecialistWorkflow = string;
export type SpecialistTab = string;
export type ScoutTab = "map" | "changes" | "simulations" | "evidence" | "runs";
export type SecurityTab = "posture" | "findings" | "zero-days" | "campaigns" | "scans";
export type ScientistTab = "programs" | "campaigns" | "experiments" | "evidence" | "runs";

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
  /** Host-issued idempotency binding for one explicit human Scout start. */
  scoutRunRequestId?: string;
  targetId?: string;
}

const SPECIALIST_CONTEXT_STRING_FIELDS = [
  "organizationId",
  "workspaceId",
  "repositoryId",
  "objectKind",
  "objectId",
  "workflow",
  "programId",
  "campaignId",
  "studyId",
  "experimentId",
  "runId",
  "scoutRunRequestId",
  "targetId",
] as const satisfies ReadonlyArray<Exclude<keyof SpecialistContext, "kind">>;

/** Filesystem roots a conversation-bound specialist may inspect without
 * making its document workspace writable. */
export function specialistReadRoots(
  context: SpecialistContext | null | undefined,
  recentProjects: string[],
): string[] {
  return context?.kind === "scout" ? recentProjects : [];
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
  entitlement: "included" | "subscription";
  modelPolicy: "included" | "specialist";
  runtime?: Readonly<{
    modelRoute: string;
  }>;
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
      !isSupportedSpecialistKind(definition.kind)
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

function catalogSpecialistKind(value: unknown): SpecialistKind {
  const kind = catalogString(value, "specialist kind");
  if (!isSupportedSpecialistKind(kind)) {
    throw new Error(`Specialist kind has no registered presentation adapter: ${kind}`);
  }
  return kind as SpecialistKind;
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
    const entitlement = manifest.entitlement;
    if (entitlement !== "included" && entitlement !== "subscription") {
      throw new Error(`Specialist manifest ${index} has an invalid entitlement`);
    }
    const modelPolicy = manifest.modelPolicy ?? "specialist";
    if (modelPolicy !== "included" && modelPolicy !== "specialist") {
      throw new Error(`Specialist manifest ${index} has an invalid model policy`);
    }
    if (!Array.isArray(manifest.tabs) || !Array.isArray(manifest.slashCommands)) {
      throw new Error(`Specialist manifest ${index} has invalid navigation`);
    }
    const runtime = manifest.runtime === undefined
      ? undefined
      : catalogObject(manifest.runtime, `Specialist manifest ${index} runtime`);
    if (
      (engine === "research_runtime" && !runtime)
      || (runtime !== undefined && (
        typeof runtime.modelRoute !== "string"
        || !runtime.modelRoute
      ))
    ) {
      throw new Error(`Specialist manifest ${index} has an invalid runtime policy`);
    }
    const skillBindings = catalogObject(
      manifest.skillBindings,
      `Specialist manifest ${index} skill bindings`,
    );
    if (Object.values(skillBindings).some((binding) => typeof binding !== "string")) {
      throw new Error(`Specialist manifest ${index} has an invalid skill binding`);
    }
    return {
      kind: catalogSpecialistKind(manifest.kind),
      version: catalogString(manifest.version, "specialist version"),
      label: catalogString(manifest.label, "specialist label"),
      headline: catalogString(manifest.headline, "specialist headline"),
      value: catalogString(manifest.value, "specialist value"),
      engine,
      entitlement,
      modelPolicy,
      ...(runtime
        ? {
          runtime: {
            modelRoute: runtime.modelRoute as string,
          },
        }
        : {}),
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

const EMPTY_SPECIALIST_CATALOG: SpecialistCatalogReceipt = {
  schemaVersion: 1,
  catalogVersion: "1.0.0",
  catalogSha256: "0".repeat(64),
  trust: {
    source: "signed_app_bundle",
    requiresSignedReleaseBinary: true,
  },
  manifests: [],
};

export const PRODUCT_SPECIALIST_CATALOG = productModule().specialistCatalog === undefined
  ? EMPTY_SPECIALIST_CATALOG
  : parseSpecialistCatalog(productModule().specialistCatalog);
export const SPECIALIST_CATALOG_SHA256 =
  PRODUCT_SPECIALIST_CATALOG.catalogSha256;

export const SPECIALIST_REGISTRY = createSpecialistRegistry(
  PRODUCT_SPECIALIST_CATALOG.manifests,
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

/** Visible label for a conversation pinned to a non-default specialist
 * workflow. Ordinary navigation returns to the default, while saved slash
 * command conversations keep their explicit mode and must make it legible. */
export function specialistWorkflowCommand(
  context: SpecialistContext | null | undefined,
): string | null {
  if (!context?.workflow) return null;
  const definition = SPECIALIST_REGISTRY.get(context.kind);
  if (!definition || context.workflow === definition.defaultWorkflow) return null;
  const command = definition.slashCommands.find(({ workflow }) => workflow === context.workflow);
  return command?.prefixes.find((prefix) => prefix.startsWith("/")) ?? null;
}

export function productSpecialistTarget(
  context: SpecialistContext | null | undefined,
  trainingEnabled = false,
): ProductSpecialistTarget | undefined {
  if (
    !context?.organizationId?.trim()
    || (context.kind !== "scout" && context.kind !== "security")
  ) return undefined;
  const definition = SPECIALIST_REGISTRY.get(context.kind);
  if (!definition || definition.engine !== "skill") return undefined;
  return {
    organizationId: context.organizationId,
    kind: context.kind,
    workflow: context.workflow || definition.defaultWorkflow,
    ...(trainingEnabled ? { trainingOptIn: true } : {}),
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
    ...(context.scoutRunRequestId?.trim()
      ? { runRequestId: context.scoutRunRequestId.trim() }
      : {}),
    ...(separator > 0
      ? {
        platform: architecture!.slice(0, separator),
        architecture: architecture!.slice(separator + 1),
      }
      : {}),
    ...(targetId?.trim() ? { targetId: targetId.trim() } : {}),
  };
}

export function newScoutRunRequestId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(32));
  return `scout-run:${Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
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
  if (!definition || !definition.runtime || !project) {
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
      modelRoute: definition.runtime.modelRoute,
      ...(advisorTrainingEnabled ? { advisorTrainingEnabled: true } : {}),
      ...(remote ? { remote } : {}),
    },
  };
}

export function isSpecialistKind(value: string): value is SpecialistKind {
  return Boolean(SPECIALIST_REGISTRY.get(value));
}

/** Accept durable specialist metadata only when its adapter is present in the
 * signed catalog loaded by this renderer. Retired or malformed contexts become
 * ordinary conversations instead of leaking into specialist navigation or a
 * provider recipe that no longer exists. */
export function registeredSpecialistContext(value: unknown): SpecialistContext | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
  const row = value as Record<string, unknown>;
  if (typeof row.kind !== "string" || !isSpecialistKind(row.kind)) return undefined;
  const fields: Array<readonly [string, string]> = [];
  for (const field of SPECIALIST_CONTEXT_STRING_FIELDS) {
    const fieldValue = row[field];
    if (fieldValue === undefined) continue;
    if (typeof fieldValue !== "string") return undefined;
    fields.push([field, fieldValue]);
  }
  return {
    kind: row.kind,
    ...Object.fromEntries(fields),
  };
}

export type SpecialistAccessState =
  | "loading"
  | "signed_out"
  | "free"
  | "ready"
  | "action_needed"
  | "organization_required"
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
    case "organization_required":
      return "Workspace required";
    case "scope_lost":
      return "Access changed";
    case "offline":
      return "Can't verify access";
  }
}

/** Local projection used while organization-bound capability access is
 * loading. The product access projection remains authoritative before
 * specialist data or actions. */
export function projectedSpecialistAccess(
  signedIn: boolean,
  access: ProductAccessProjection | null,
  kind: SpecialistKind,
): SpecialistAccessState {
  if (!signedIn) return "signed_out";
  if (SPECIALIST_REGISTRY.get(kind)?.entitlement === "included") return "ready";
  const capability = capabilityAccess(access, kind);
  if (!capability || capability.availability === "unknown") return "loading";
  if (capability.availability === "available") return "ready";
  return ["coverage_action_needed", "coverage_unavailable", "usage_limited", "past_due"]
    .includes(capability.reason)
    ? "action_needed"
    : "free";
}

/** A rejected product access request is terminal for this attempt. Preserve
 * definitive local/catalog states, but never present an unknown projection as
 * an endless access check. */
export function specialistAccessAfterProductFailure(
  projected: SpecialistAccessState,
  failed: boolean,
): SpecialistAccessState {
  return failed && projected === "loading" ? "offline" : projected;
}

/** Keep already verified access distinct from a later specialist-data
 * failure. A failed overview/query is a canvas error, not evidence that the
 * user's paid coverage became unverifiable. */
export function specialistAccessAfterLoadFailure(
  entitlementVerified: boolean,
): Extract<SpecialistAccessState, "ready" | "offline"> {
  return entitlementVerified ? "ready" : "offline";
}

/** Included specialists are authorized by the signed product catalog. They do
 * not have a subscription entitlement or organization dashboard to verify. */
export function specialistNeedsEntitlementVerification(
  entitlement: SpecialistDefinition["entitlement"],
): boolean {
  return entitlement === "subscription";
}

export function specialistAccessCopy(
  state: SpecialistAccessState,
  kind: SpecialistKind,
): {
  title: string;
  detail: string;
  action: "sign_in" | "product_action" | "retry" | "setup_workspace" | null;
  actionLabel?: string;
} {
  const brand = productModule().branding;
  const definition = SPECIALIST_REGISTRY.get(kind);
  if (!definition) {
    return {
      title: "Unknown specialist",
      detail: `This specialist is not registered in this ${brand.shortName} build.`,
      action: null,
    };
  }
  const label = definition.label;
  const productCopy = productModule().specialistAccessCopy;
  if (productCopy) {
    return productCopy({ state, kind, label, value: definition.value });
  }
  switch (state) {
    case "loading":
      return { title: `Checking ${label} access…`, detail: "This usually takes a moment.", action: null };
    case "signed_out":
      return {
        title: `Sign in to use ${brand.shortName} ${label}`,
        detail: `${definition.value} Specialist work is saved to your ${brand.shortName} account.`,
        action: "sign_in",
      };
    case "action_needed":
      return {
        title: `${label} access needs attention`,
        detail: "Review this capability in the active product account. Saved work remains available.",
        action: "product_action",
        actionLabel: "Review access",
      };
    case "organization_required":
      return {
        title: `Join a workspace to use ${label}`,
        detail: `${label} journals and verified artifacts are workspace-scoped. Join or create a Clark workspace, then retry.`,
        action: "setup_workspace",
        actionLabel: "Set up workspace",
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
        title: `${label} is unavailable`,
        detail: `This capability is not available in the current ${brand.shortName} configuration. ${definition.value}`,
        action: "product_action",
        actionLabel: "Review access",
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
