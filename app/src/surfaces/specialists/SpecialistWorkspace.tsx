import { lazy, Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import {
  Building2,
  ChevronDown,
  MessageSquareText,
  Moon,
  PanelRightClose,
  PanelRightOpen,
  Plus,
  Settings,
  Sun,
} from "lucide-react";
import { useSessionStore } from "../../store/sessionStore";
import { useSpecialistStore } from "../../store/specialistStore";
import { useProductAccess } from "../../lib/useProductAccess";
import { capabilityAccess } from "../../lib/productAccess";
import { openExternal } from "../../lib/externalLinks";
import {
  SPECIALISTS,
  projectedSpecialistAccess,
  specialistAccessAfterLoadFailure,
  specialistAccessBadge,
  type ScoutTab,
  type SecurityTab,
  type ScientistTab,
  type RsiTab,
  type SpecialistKind,
} from "../../lib/specialists";
import {
  specialistEntitlement,
  specialistOrganizations,
  specialistCreateOrganization,
  specialistQuery,
  specialistCreateWorkspace,
  specialistCreateSecurityCampaign,
  type ScoutChange,
  type ScoutSimulation,
  type ScoutSnapshotEntry,
  type ScoutWorkspace,
  type SecurityFinding,
  type SecurityCampaign,
  type SecurityPosture,
  type SecurityRepository,
  type SecurityScan,
  type SpecialistOrganization,
  type ScienceArtifactSegment,
  type ResearchOverview,
  type RsiOverview,
} from "../../lib/specialistCloud";
import { cloudCreds, type CloudCreds } from "../../lib/cloudHistory";
import { syncSecurityInsights } from "../../lib/securityCloud";
import { saveSecurityScanPdf } from "../../lib/securityReport";
import type { SecurityScanRecord } from "../../core-bridge/types";
import { cn } from "../../lib/cn";
import { RISE, accessibleMotion } from "../../lib/motion";
import { Composer } from "../Composer";
import { GoalStatusRail } from "../GoalStatusRail";
import { UpdatePill } from "../TopBar";
import { PanelErrorBoundary } from "../../components/PanelErrorBoundary";
import { ScoutCanvas } from "./ScoutCanvas";
import { SecurityCanvas } from "./SecurityCanvas";
import { ScientistCanvas } from "./ScientistCanvas";
import { RsiCanvas } from "./RsiCanvas";
import { CanvasStatus } from "./SpecialistPrimitives";
import { SpecialistWelcome, type SpecialistStarter } from "./SpecialistWelcome";
import { SpecialistAccessGate } from "./SpecialistAccessGate";
import { SpecWorkspace } from "./SpecWorkspace";

const Conversation = lazy(() =>
  import("../Conversation").then((module) => ({ default: module.Conversation })),
);

interface SpecialistData {
  workspaces: ScoutWorkspace[];
  entries: ScoutSnapshotEntry[];
  changes: ScoutChange[];
  simulations: ScoutSimulation[];
  posture: SecurityPosture | null;
  repositories: SecurityRepository[];
  findings: SecurityFinding[];
  candidates: SecurityFinding[];
  scans: SecurityScan[];
  campaigns: SecurityCampaign[];
  localSecurityScans: SecurityScanRecord[];
  researchOverview: ResearchOverview | null;
  rsiOverview: RsiOverview | null;
  scienceArtifacts: ScienceArtifactSegment[];
}

const EMPTY_DATA: SpecialistData = {
  workspaces: [],
  entries: [],
  changes: [],
  simulations: [],
  posture: null,
  repositories: [],
  findings: [],
  candidates: [],
  scans: [],
  campaigns: [],
  localSecurityScans: [],
  researchOverview: null,
  rsiOverview: null,
  scienceArtifacts: [],
};

function previewAccess(): "paid" | "free" | null {
  if (!import.meta.env.DEV || typeof window === "undefined") return null;
  const value = new URLSearchParams(window.location.search).get("specialistPreview");
  return value === "paid" || value === "free" ? value : null;
}

function previewCredentials(): CloudCreds {
  return { accountScope: "preview:specialist" };
}

function ContextualConversation({ kind }: { kind: SpecialistKind }) {
  const session = useSessionStore((state) => state.session);
  const setComposerPrefill = useSessionStore((state) => state.setComposerPrefill);
  const setTab = useSpecialistStore((state) => state.setTab);
  const setContext = useSpecialistStore((state) => state.setContext);
  const reduceMotion = useReducedMotion();
  const definition = SPECIALISTS[kind];
  const start = (starter: SpecialistStarter) => {
    setContext({ workflow: starter.workflow });
    setTab(starter.tab);
    setComposerPrefill(starter.prompt);
  };

  return (
    <section
      data-qa={`specialist-conversation-${kind}`}
      aria-label={`${definition.label} contextual conversation`}
      className="flex h-full min-h-0 min-w-0 flex-col bg-bg"
    >
      <AnimatePresence initial={false} mode="wait">
        {session ? (
          <m.div
            key={`conversation:${session.id}`}
            {...accessibleMotion(RISE, reduceMotion)}
            className="flex min-h-0 flex-1 flex-col overflow-hidden"
          >
            <PanelErrorBoundary title={`${definition.label} conversation needs to restart`} resetKey={session.id}>
              <Suspense fallback={<div className="h-full min-h-0" />}>
                <Conversation />
              </Suspense>
            </PanelErrorBoundary>
          </m.div>
        ) : (
          <m.div
            key={`${kind}:welcome`}
            {...accessibleMotion(RISE, reduceMotion)}
            className="min-h-0 flex-1 overflow-y-auto px-5 py-6"
          >
            <SpecialistWelcome kind={kind} onStart={start} />
          </m.div>
        )}
      </AnimatePresence>
      {session && <GoalStatusRail />}
      <Composer />
    </section>
  );
}

export function SpecialistWorkspace({
  dark,
  onToggleTheme,
}: {
  dark: boolean;
  onToggleTheme: () => void;
}) {
  const active = useSpecialistStore((state) => state.active) ?? "scout";
  const tabs = useSpecialistStore((state) => state.tabs);
  const contexts = useSpecialistStore((state) => state.contexts);
  const setTab = useSpecialistStore((state) => state.setTab);
  const setContext = useSpecialistStore((state) => state.setContext);
  const openSpecialist = useSpecialistStore((state) => state.open);
  const auth = useSessionStore((state) => state.auth);
  const bridge = useSessionStore((state) => state.bridge);
  const securityCompletionKey = useSessionStore((state) => active === "security"
    ? Object.values(state.snapshot.runs)
        .filter((run) => run.status === "done")
        .map((run) => run.id)
        .join("\u0000")
    : "");
  const boundConversation = useSessionStore((state) => state.session
    ? state.conversations.find((conversation) => conversation.id === state.session?.id)
    : undefined);
  const boundContext = boundConversation?.specialist;
  const productAccess = useProductAccess(true);
  const setComposerPrefill = useSessionStore((state) => state.setComposerPrefill);
  const setSettingsOpen = useSessionStore((state) => state.setSettingsOpen);
  const configuredCwd = useSessionStore(
    (state) => state.activeProjectRoot ?? state.localSettings.cwd,
  );
  const cwd = boundConversation?.remoteHost
    ? ""
    : boundConversation?.project ?? configuredCwd;
  const [organizations, setOrganizations] = useState<SpecialistOrganization[]>([]);
  const [data, setData] = useState<SpecialistData>(EMPTY_DATA);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [serverAccess, setServerAccess] = useState<"unknown" | "ready" | "free" | "action_needed" | "organization_required" | "scope_lost" | "offline">("unknown");
  const [mobilePane, setMobilePane] = useState<"chat" | "canvas">("chat");
  const [canvasOpen, setCanvasOpen] = useState(false);
  const [creatingWorkspace, setCreatingWorkspace] = useState(false);
  const [organizationDialogOpen, setOrganizationDialogOpen] = useState(false);
  const [creatingOrganization, setCreatingOrganization] = useState(false);
  const [organizationName, setOrganizationName] = useState("");
  const [organizationDomain, setOrganizationDomain] = useState("");
  const definition = SPECIALISTS[active];
  const context = boundContext?.kind === active ? boundContext : contexts[active] ?? { kind: active };
  const preview = previewAccess();
  const projected = preview
    ? preview === "paid" ? "ready" : "free"
    : projectedSpecialistAccess(Boolean(auth), productAccess.access, active);
  const accessCapability = capabilityAccess(productAccess.access, active);
  const access = projected === "ready"
    ? serverAccess === "unknown" ? "loading" : serverAccess
    : projected;
  const credentials = cloudCreds(auth) ?? (preview === "paid" ? previewCredentials() : null);

  const clearSensitiveData = useCallback(() => {
    setData(EMPTY_DATA);
    setOrganizations([]);
  }, []);

  const load = useCallback(async () => {
    if (active === "spec") {
      setServerAccess("ready");
      setData(EMPTY_DATA);
      setError(null);
      return;
    }
    if (projected !== "ready" || !credentials) {
      clearSensitiveData();
      setServerAccess(projected === "action_needed" ? "action_needed" : "free");
      return;
    }
    setLoading(true);
    setError(null);
    let entitlementVerified = false;
    try {
      const orgs = (await specialistOrganizations(credentials)).filter((organization) => organization.status === "active");
      if (
        boundContext?.kind === active
        && boundContext.organizationId
        && !orgs.some((item) => item.id === boundContext.organizationId)
      ) {
        clearSensitiveData();
        setServerAccess("scope_lost");
        return;
      }
      const organization = orgs.find((item) => item.id === context.organizationId)
        ?? (active === "scout" ? undefined : orgs[0]);
      setOrganizations(orgs);
      if (!organization) {
        setData(EMPTY_DATA);
        // Scout owns an explicit create/select scope flow in this workspace.
        // Do not hide that human action behind the generic access gate.
        setServerAccess(active === "scout" ? "ready" : orgs.length === 0 ? "organization_required" : "ready");
        return;
      }
      const entitlement = await specialistEntitlement(credentials, active, organization.id);
      if (!entitlement.allowed) {
        clearSensitiveData();
        setServerAccess(entitlement.state);
        return;
      }
      entitlementVerified = true;
      setServerAccess("ready");
      if (active !== "scout" && context.organizationId !== organization.id) {
        setContext({ organizationId: organization.id });
      }
      if (active === "scout") {
        const workspaces = await specialistQuery<ScoutWorkspace[]>(
          credentials, active, "scout_workspaces", organization.id,
        );
        const workspace = workspaces.find((item) => item.id === context.workspaceId);
        if (!workspace) {
          if (boundContext?.kind === active && context.workspaceId) {
            clearSensitiveData();
            setServerAccess("scope_lost");
            return;
          }
          setData({ ...EMPTY_DATA, workspaces });
          return;
        }
        const [snapshot, changes, simulations] = await Promise.all([
          specialistQuery<{ entries: ScoutSnapshotEntry[] }>(
            credentials, active, "scout_snapshot", organization.id, workspace.id,
          ),
          specialistQuery<{ changes: ScoutChange[] }>(
            credentials, active, "scout_changes", organization.id, workspace.id,
          ),
          specialistQuery<ScoutSimulation[]>(
            credentials, active, "scout_simulations", organization.id, workspace.id,
          ),
        ]);
        setData({
          ...EMPTY_DATA,
          workspaces,
          entries: snapshot.entries,
          changes: changes.changes,
          simulations,
        });
      } else if (active === "security") {
        const sync = await syncSecurityInsights(
          credentials,
          organization.id,
          cwd,
        );
        if (sync?.failedCount) {
          const firstFailure = sync.scans.find((scan) => scan.status === "failed")?.message;
          throw new Error(firstFailure
            ? `Security scan sync failed: ${firstFailure}`
            : `${sync.failedCount} Security scan sync attempt${sync.failedCount === 1 ? "" : "s"} failed.`);
        }
        const [posture, repositories, findings, candidates, campaigns, localSecurityScans] = await Promise.all([
          specialistQuery<SecurityPosture>(credentials, active, "security_posture", organization.id),
          specialistQuery<{ data: SecurityRepository[] }>(credentials, active, "security_repositories", organization.id),
          specialistQuery<{ data: SecurityFinding[] }>(credentials, active, "security_findings", organization.id),
          specialistQuery<{ data: SecurityFinding[] }>(credentials, active, "security_candidates", organization.id),
          specialistQuery<{ data: SecurityCampaign[] }>(credentials, active, "security_campaigns", organization.id),
          cwd && bridge?.listSecurityScans
            ? bridge.listSecurityScans(cwd)
            : Promise.resolve([]),
        ]);
        const repository = repositories.data.find((item) => item.repositoryId === context.repositoryId)
          ?? repositories.data[0];
        const scans = repository
          ? await specialistQuery<{ data: SecurityScan[] }>(
              credentials, active, "security_scans", organization.id, undefined, repository.repositoryId,
            )
          : { data: [] };
        if (repository && context.repositoryId !== repository.repositoryId) {
          setContext({ repositoryId: repository.repositoryId });
        }
        const localScanIdByPlatformId = new Map(
          (sync?.scans ?? []).flatMap((item) => (
            item.platformScanId && item.localScanId
              ? [[item.platformScanId, item.localScanId] as const]
              : []
          )),
        );
        const localScanIdByClientId = new Map<string, string>(
          localSecurityScans.flatMap((record) => record.seal?.bundleDigest
            ? [[
                `scan:desktop:${record.seal.bundleDigest.slice(0, 32)}`,
                record.bundle.scanId,
              ] as const]
            : []),
        );
        const decoratedScans = scans.data.map((scan) => ({
          ...scan,
          localScanId: localScanIdByPlatformId.get(scan.id)
            ?? (scan.clientScanId ? localScanIdByClientId.get(scan.clientScanId) : undefined)
            ?? null,
        }));
        setData({
          ...EMPTY_DATA,
          posture,
          repositories: repositories.data,
          findings: findings.data,
          candidates: candidates.data,
          scans: decoratedScans,
          campaigns: campaigns.data,
          localSecurityScans,
        });
      } else if (active === "scientist") {
        const [researchOverview, scienceArtifacts] = await Promise.all([
          specialistQuery<ResearchOverview>(
            credentials,
            active,
            "scientist_overview",
            organization.id,
          ),
          specialistQuery<ScienceArtifactSegment[]>(
            credentials,
            active,
            "scientist_artifacts",
            organization.id,
          ),
        ]);
        setData({ ...EMPTY_DATA, researchOverview, scienceArtifacts });
      } else if (active === "rsi") {
        const [rsiOverview, scienceArtifacts] = await Promise.all([
          specialistQuery<RsiOverview>(
            credentials,
            active,
            "rsi_overview",
            organization.id,
          ),
          specialistQuery<ScienceArtifactSegment[]>(
            credentials,
            active,
            "rsi_artifacts",
            organization.id,
          ),
        ]);
        setData({ ...EMPTY_DATA, rsiOverview, scienceArtifacts });
      } else {
        throw new Error(`No data adapter is registered for specialist ${active}`);
      }
    } catch (cause) {
      setServerAccess(specialistAccessAfterLoadFailure(entitlementVerified));
      clearSensitiveData();
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }, [
    active,
    boundContext,
    bridge,
    clearSensitiveData,
    context.organizationId,
    context.repositoryId,
    context.workspaceId,
    credentials?.accountScope,
    cwd,
    projected,
    setContext,
  ]);

  useEffect(() => {
    setServerAccess("unknown");
    void load();
  }, [load, securityCompletionKey]);

  useEffect(() => bridge?.onSpecialistProjectionPublished?.((receipt) => {
    if (
      receipt.specialist === active
      && (!context.organizationId || receipt.organizationId === context.organizationId)
    ) {
      void load();
    }
  }), [active, bridge, context.organizationId, load]);

  useEffect(() => {
    if (projected !== "ready") clearSensitiveData();
  }, [clearSensitiveData, projected]);

  // Workspace creation is always a visible human action. Session startup never
  // creates or silently selects cloud authority on the user's behalf.
  const createWorkspace = useCallback(async () => {
    if (!credentials || !context.organizationId?.trim()) return;
    setCreatingWorkspace(true);
    setError(null);
    try {
      const created = await specialistCreateWorkspace(
        credentials,
        context.organizationId,
        "Scout workspace",
      );
      setContext({ workspaceId: created.id });
      await load();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setCreatingWorkspace(false);
    }
  }, [context.organizationId, credentials, load, setContext]);

  const createOrganization = useCallback(async () => {
    if (!credentials || !organizationName.trim() || !organizationDomain.trim()) return;
    setCreatingOrganization(true);
    setError(null);
    try {
      const created = await specialistCreateOrganization(
        credentials,
        organizationName.trim(),
        organizationDomain.trim(),
      );
      setOrganizations((current) => [
        created,
        ...current.filter((organization) => organization.id !== created.id),
      ]);
      setContext({ organizationId: created.id, workspaceId: undefined, repositoryId: undefined });
      setOrganizationDialogOpen(false);
      setOrganizationName("");
      setOrganizationDomain("");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setCreatingOrganization(false);
    }
  }, [credentials, organizationDomain, organizationName, setContext]);

  const activeWorkspace = useMemo(
    () => data.workspaces.find((workspace) => workspace.id === context.workspaceId) ?? null,
    [context.workspaceId, data.workspaces],
  );

  if (active === "spec") return <SpecWorkspace />;

  const canvas = (
    <div className="min-h-0 flex-1 overflow-y-auto bg-bg-secondary/30">
      <CanvasStatus loading={(loading || creatingWorkspace) && serverAccess === "ready"} error={error} onRetry={() => void load()} />
      {!loading && !creatingWorkspace && !error && serverAccess === "ready" && (
        active === "scout" ? (
          <ScoutCanvas
            tab={tabs[active] as ScoutTab}
            workspace={activeWorkspace}
            entries={data.entries}
            changes={data.changes}
            simulations={data.simulations}
            onStartSimulation={() => {
              setContext({ workflow: "scout:scout" });
              setComposerPrefill("Simulate an important failure in the mapped system, show the evidence-backed blast radius, and identify recovery gaps.");
              setMobilePane("chat");
            }}
            onSelectEntry={(entry) => {
              const name = entry.event.fact.attributes.name;
              const label = typeof name === "string" && name.trim() ? name : entry.object_id;
              setContext({
                workspaceId: activeWorkspace?.id,
                objectKind: entry.object_kind,
                objectId: entry.object_id,
                workflow: "scout:scout",
              });
              setComposerPrefill(`Explain “${label}”, show its supporting evidence and relationships, and assess its operational impact.`);
              setMobilePane("chat");
            }}
          />
        ) : active === "security" ? (
          <SecurityCanvas
            tab={tabs[active] as SecurityTab}
            posture={data.posture}
            repositories={data.repositories}
            findings={data.findings}
            candidates={data.candidates}
            scans={data.scans}
            campaigns={data.campaigns}
            onSaveScanPdf={(scan) => saveSecurityScanPdf(
              scan,
              data.localSecurityScans.find((record) => record.bundle.scanId === scan.localScanId),
            )}
            onSelectTab={(tab) => setTab(tab)}
            onSelectRepository={(repository) => {
              setContext({ repositoryId: repository.repositoryId });
              setTab("scans");
              setMobilePane("canvas");
            }}
            onSelectFinding={(finding) => {
              setContext({
                repositoryId: finding.repositoryId,
                objectKind: "security_finding",
                objectId: finding.id,
                workflow: "security:security-scan",
              });
              setComposerPrefill(`Investigate the ${finding.currentSeverity} finding “${finding.title}”, show its evidence, and recommend the safest remediation.`);
              setMobilePane("chat");
            }}
            onStartScan={() => {
              setContext({ workflow: "security:security-scan" });
              setComposerPrefill("Scan the selected repository, validate exploitable findings, and show the supporting evidence.");
              setMobilePane("chat");
            }}
            onResearchCandidate={() => {
              setContext({ workflow: "security:security-deep" });
              setComposerPrefill("Research a novel vulnerability candidate in the selected repository and separate confirmed evidence from unresolved hypotheses.");
              setMobilePane("chat");
            }}
            onCreateCampaign={async (title, description, findingIds) => {
              if (!credentials || !context.organizationId) {
                throw new Error("Security scanner organization context is unavailable");
              }
              await specialistCreateSecurityCampaign(
                credentials,
                context.organizationId,
                title,
                description,
                findingIds,
              );
              await load();
            }}
          />
        ) : active === "scientist" ? (
          <ScientistCanvas
            tab={tabs[active] as ScientistTab}
            overview={data.researchOverview}
            artifacts={data.scienceArtifacts}
          />
        ) : (
          <RsiCanvas
            tab={tabs[active] as RsiTab}
            overview={data.rsiOverview}
            artifacts={data.scienceArtifacts}
          />
        )
      )}
    </div>
  );

  return (
    <div data-qa={`specialist-workspace-${active}`} className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-bg">
      <header className="flex min-h-16 shrink-0 items-center gap-4 px-5 py-2.5">
        <div className="min-w-0">
          <h1 className="font-serif text-2xl font-semibold tracking-[-0.03em] text-ink">the agent {definition.label}</h1>
          <p className="mt-0.5 line-clamp-2 max-w-2xl text-xs leading-4 text-ink-muted">{definition.value}</p>
        </div>
        <div className="ml-auto flex items-center gap-2">
          <UpdatePill />
          <button
            type="button"
            data-qa={`specialist-show-insights-${active}`}
            onClick={() => setCanvasOpen((open) => !open)}
            aria-label={canvasOpen ? `Hide ${definition.label} sidebar` : `Show ${definition.label} sidebar`}
            aria-expanded={canvasOpen}
            className="hidden h-9 items-center gap-2 rounded-xl px-3 text-xs font-medium text-ink-muted transition hover:bg-bg-hover hover:text-ink xl:flex"
          >
            {canvasOpen ? <PanelRightClose className="size-4" /> : <PanelRightOpen className="size-4" />}
            {canvasOpen ? "Hide insights" : "Show insights"}
          </button>
          {organizations.length > 0 && serverAccess === "ready" && (
            <label className="relative hidden md:block">
              <span className="sr-only">Organization</span>
              <Building2 className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-ink-faint" />
              <select
                value={active === "scout" ? context.organizationId ?? "" : context.organizationId ?? organizations[0]?.id}
                onChange={(event) => setContext({ organizationId: event.target.value, workspaceId: undefined, repositoryId: undefined })}
                disabled={boundContext?.kind === active}
                title={boundContext?.kind === active ? "Start a new specialist conversation to change organization" : undefined}
                className="h-9 appearance-none rounded-xl bg-bg-secondary pl-8 pr-8 text-xs font-medium text-ink-secondary outline-none transition focus:ring-2 focus:ring-accent/20"
              >
                {active === "scout" && <option value="">Choose organization…</option>}
                {organizations.map((organization) => (
                  <option key={organization.id} value={organization.id}>{organization.name}</option>
                ))}
              </select>
              <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 size-3.5 -translate-y-1/2 text-ink-faint" />
            </label>
          )}
          {active === "scout" && serverAccess === "ready" && boundContext?.kind !== active && (
            <button
              type="button"
              onClick={() => setOrganizationDialogOpen(true)}
              aria-label="Create organization"
              title="Create organization"
              className="grid size-9 place-items-center rounded-xl text-ink-muted transition hover:bg-bg-hover hover:text-ink"
            >
              <Plus className="size-4" />
            </button>
          )}
          {active === "scout" && context.organizationId && organizations.length > 0 && serverAccess === "ready" && (
            data.workspaces.length > 0 ? (
              <label className="relative hidden md:block">
                <span className="sr-only">Workspace</span>
                <select
                  value={context.workspaceId ?? ""}
                  onChange={(event) => setContext({ workspaceId: event.target.value || undefined })}
                  disabled={boundContext?.kind === active}
                  title={boundContext?.kind === active ? "Start a new specialist conversation to change workspace" : undefined}
                  className="h-9 appearance-none rounded-xl bg-bg-secondary pl-8 pr-8 text-xs font-medium text-ink-secondary outline-none transition focus:ring-2 focus:ring-accent/20"
                >
                  <option value="">Choose workspace…</option>
                  {data.workspaces.map((workspace) => (
                    <option key={workspace.id} value={workspace.id}>{workspace.display_name}</option>
                  ))}
                </select>
                <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 size-3.5 -translate-y-1/2 text-ink-faint" />
              </label>
            ) : boundContext?.kind !== active ? (
              <button
                type="button"
                onClick={() => void createWorkspace()}
                disabled={creatingWorkspace}
                className="flex h-9 items-center gap-1.5 rounded-xl bg-accent px-3 text-xs font-semibold text-on-accent transition hover:bg-accent/90 disabled:opacity-50"
              >
                <Plus className="size-3.5" />
                {creatingWorkspace ? "Creating…" : "Create workspace"}
              </button>
            ) : null
          )}
          <span className={cn(
            "hidden rounded-full px-2.5 py-1 text-xs font-medium sm:inline-flex",
            access === "ready" ? "bg-success/10 text-success" : "bg-accent-soft text-accent",
          )}>
            {specialistAccessBadge(access)}
          </span>
          <button
            type="button"
            onClick={() => setSettingsOpen(true)}
            aria-label="Settings"
            className="grid size-9 place-items-center rounded-xl text-ink-muted transition hover:bg-bg-hover hover:text-ink"
          >
            <Settings className="size-4" />
          </button>
          <button
            type="button"
            onClick={onToggleTheme}
            aria-label={dark ? "Switch to light theme" : "Switch to dark theme"}
            className="grid size-9 place-items-center rounded-xl text-ink-muted transition hover:bg-bg-hover hover:text-ink"
          >
            {dark ? <Sun className="size-4" /> : <Moon className="size-4" />}
          </button>
        </div>
      </header>

      {organizationDialogOpen && (
        <div className="fixed inset-0 z-50 grid place-items-center bg-black/40 p-4" role="presentation">
          <form
            role="dialog"
            aria-modal="true"
            aria-labelledby="create-scout-organization-title"
            onSubmit={(event) => {
              event.preventDefault();
              void createOrganization();
            }}
            className="w-full max-w-md rounded-2xl border border-border bg-bg-elevated p-5 shadow-xl"
          >
            <h2 id="create-scout-organization-title" className="text-base font-semibold text-ink">Create Scout organization</h2>
            <p className="mt-1 text-xs leading-5 text-ink-muted">This creates the explicit tenant boundary. Scout will still wait for you to create a workspace and press Start run.</p>
            <label className="mt-4 block text-xs font-medium text-ink-secondary">
              Organization name
              <input
                value={organizationName}
                onChange={(event) => setOrganizationName(event.target.value)}
                autoFocus
                className="mt-1.5 h-10 w-full rounded-xl border border-border bg-bg px-3 text-sm text-ink outline-none focus:border-accent"
              />
            </label>
            <label className="mt-3 block text-xs font-medium text-ink-secondary">
              Company domain
              <input
                value={organizationDomain}
                onChange={(event) => setOrganizationDomain(event.target.value)}
                placeholder="example.com"
                className="mt-1.5 h-10 w-full rounded-xl border border-border bg-bg px-3 text-sm text-ink outline-none focus:border-accent"
              />
            </label>
            <div className="mt-5 flex justify-end gap-2">
              <button type="button" onClick={() => setOrganizationDialogOpen(false)} className="rounded-xl px-3 py-2 text-xs font-medium text-ink-muted hover:bg-bg-hover">Cancel</button>
              <button
                type="submit"
                disabled={creatingOrganization || !organizationName.trim() || !organizationDomain.trim()}
                className="rounded-xl bg-accent px-3 py-2 text-xs font-semibold text-on-accent disabled:opacity-50"
              >
                {creatingOrganization ? "Creating…" : "Create organization"}
              </button>
            </div>
          </form>
        </div>
      )}

      {access !== "ready" ? (
        <SpecialistAccessGate
          key={`${active}:${access}`}
          kind={active}
          state={access}
          onProductAction={() => {
            if (accessCapability?.actionUrl) void openExternal(accessCapability.actionUrl);
            else setSettingsOpen(true);
          }}
          onWorkspaceSetup={() => openSpecialist("scout")}
          onRetry={() => {
            void productAccess.reload().catch(() => undefined);
            setServerAccess("unknown");
            void load();
          }}
        />
      ) : (
          <div className="flex min-h-0 min-w-0 flex-1 flex-col">
            <div className={cn(
              "flex h-10 shrink-0 items-end px-3",
              canvasOpen ? "xl:justify-end" : "xl:h-0 xl:overflow-hidden",
            )}>
              <div className={cn("hidden h-full items-end xl:flex", !canvasOpen && "xl:hidden")}>
                {definition.tabs.map((tab) => (
                  <button
                    key={tab.id}
                    data-qa={`specialist-tab-${active}-${tab.id}`}
                    type="button"
                    onClick={() => setTab(tab.id)}
                    className={cn(
                      "relative h-10 px-3 text-xs font-medium transition",
                      tabs[active] === tab.id ? "text-accent" : "text-ink-muted hover:text-ink",
                    )}
                  >
                    {tab.label}
                    {tabs[active] === tab.id && <span className="absolute inset-x-2 bottom-0 h-0.5 rounded-full bg-accent" />}
                  </button>
                ))}
              </div>
              <div className="flex h-full w-full items-center gap-1 overflow-x-auto [scrollbar-width:none] xl:hidden [&::-webkit-scrollbar]:hidden">
                <button
                  type="button"
                  onClick={() => setMobilePane("chat")}
                  className={cn(
                    "flex shrink-0 items-center justify-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium",
                    mobilePane === "chat" ? "bg-accent-soft text-accent" : "text-ink-muted",
                  )}
                >
                  <MessageSquareText className="size-3.5" /> Chat
                </button>
                {definition.tabs.map((tab) => (
                  <button
                    key={tab.id}
                    data-qa={`specialist-tab-${active}-${tab.id}`}
                    type="button"
                    onClick={() => {
                      setTab(tab.id);
                      setMobilePane("canvas");
                    }}
                    className={cn(
                      "shrink-0 rounded-lg px-2.5 py-1.5 text-xs font-medium",
                      mobilePane === "canvas" && tabs[active] === tab.id
                        ? "bg-accent-soft text-accent"
                        : "text-ink-muted",
                    )}
                  >
                    {tab.label}
                  </button>
                ))}
              </div>
            </div>
            <div className={cn(
              "grid min-h-0 min-w-0 flex-1",
              canvasOpen && "xl:grid-cols-[minmax(32rem,1fr)_clamp(22rem,34vw,30rem)]",
            )}>
              <div className={cn("min-h-0 min-w-0", mobilePane !== "chat" && "hidden xl:block")}>
                <ContextualConversation kind={active} />
              </div>
              <section
                data-qa={`specialist-canvas-${active}`}
                aria-label={`${definition.label} canvas`}
                className={cn(
                  "flex min-h-0 min-w-0 flex-col bg-bg",
                  mobilePane !== "canvas" && "hidden",
                  canvasOpen && "xl:flex",
                )}
              >
                {canvas}
              </section>
            </div>
          </div>
      )}
    </div>
  );
}
