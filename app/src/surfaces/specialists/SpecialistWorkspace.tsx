import { useCallback, useEffect, useState } from "react";
import {
  AlertTriangle,
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
import { productModule } from "../../product/productModule";
import { capabilityAccess } from "../../lib/productAccess";
import { openExternal } from "../../lib/externalLinks";
import {
  SPECIALISTS,
  projectedSpecialistAccess,
  specialistAccessAfterProductFailure,
  specialistAccessAfterLoadFailure,
  specialistAccessBadge,
  specialistNeedsEntitlementVerification,
  type ScoutTab,
  type SecurityTab,
  type ScientistTab,
} from "../../lib/specialists";
import {
  specialistEntitlement,
  specialistOrganizations,
  specialistCreateOrganization,
  specialistQuery,
  specialistSetupCompanyScout,
  specialistCreateSecurityCampaign,
  companyScoutMap,
  type ScoutChange,
  type ScoutSimulation,
  type ScoutSnapshotEntry,
  type CompanyScoutMap,
  type SecurityFinding,
  type SecurityCampaign,
  type SecurityPosture,
  type SecurityRepository,
  type SecurityScan,
  type SpecialistOrganization,
  type ScienceArtifactSegment,
  type ResearchOverview,
} from "../../lib/specialistCloud";
import { cloudCreds, type CloudCreds } from "../../lib/cloudHistory";
import { syncSecurityInsights } from "../../lib/securityCloud";
import { saveSecurityScanPdf } from "../../lib/securityReport";
import type { SecurityScanRecord } from "../../core-bridge/types";
import { cn } from "../../lib/cn";
import { codeKeyAccountBinding } from "../../lib/account";
import { UpdatePill } from "../TopBar";
import { ScoutCanvas } from "./ScoutCanvas";
import { SecurityCanvas } from "./SecurityCanvas";
import { ScientistCanvas } from "./ScientistCanvas";
import { CanvasStatus } from "./SpecialistPrimitives";
import { SpecialistAccessGate } from "./SpecialistAccessGate";
import { ScoutOrganizationDialog } from "./ScoutOrganizationDialog";
import { ScoutScopeDialog } from "./ScoutScopeDialog";
import { ContextualConversation } from "./ContextualConversation";
import {
  CompanyScoutSetupControl,
  CompanyScoutSetupNotice,
  type CompanyScoutSetupNoticeValue,
} from "./ScoutCompanySetup";

interface SpecialistData {
  companyMap: CompanyScoutMap | null;
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
  scienceArtifacts: ScienceArtifactSegment[];
}

const EMPTY_DATA: SpecialistData = {
  companyMap: null,
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
  const scoutScopeOpen = useSpecialistStore((state) => state.scoutScopeOpen);
  const setScoutScopeOpen = useSpecialistStore((state) => state.setScoutScopeOpen);
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
  const productAccess = useProductAccess(Boolean(auth), codeKeyAccountBinding(auth));
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
  const [projectionWarning, setProjectionWarning] = useState<string | null>(null);
  const [serverAccess, setServerAccess] = useState<"unknown" | "ready" | "free" | "action_needed" | "organization_required" | "scope_lost" | "offline">("unknown");
  const [mobilePane, setMobilePane] = useState<"chat" | "canvas">("chat");
  const [canvasOpen, setCanvasOpen] = useState(false);
  const [settingUpCompanyScout, setSettingUpCompanyScout] = useState(false);
  const [companyScoutSetupNotice, setCompanyScoutSetupNotice] = useState<CompanyScoutSetupNoticeValue | null>(null);
  const [organizationDialogOpen, setOrganizationDialogOpen] = useState(false);
  const [creatingOrganization, setCreatingOrganization] = useState(false);
  const [organizationName, setOrganizationName] = useState("");
  const [organizationDomain, setOrganizationDomain] = useState("");
  const definition = SPECIALISTS[active];
  const supportsCanvas = active === "scout" || active === "security" || active === "scientist";
  const context = boundContext?.kind === active ? boundContext : contexts[active] ?? { kind: active };
  const preview = previewAccess();
  const productProjection = preview
    ? specialistNeedsEntitlementVerification(definition.entitlement)
      ? preview === "paid" ? "ready" : "free"
      : "ready"
    : projectedSpecialistAccess(Boolean(auth), productAccess.access, active);
  const projected = specialistAccessAfterProductFailure(
    productProjection,
    Boolean(productAccess.error) && !productAccess.loading,
  );
  const accessCapability = capabilityAccess(productAccess.access, active);
  const access = projected === "ready"
    ? serverAccess === "unknown" ? "loading" : serverAccess
    : projected;
  const credentials = cloudCreds(auth) ?? (preview ? previewCredentials() : null);

  const clearSensitiveData = useCallback(() => {
    setData(EMPTY_DATA);
    setOrganizations([]);
  }, []);

  const selectOrganization = useCallback((organizationId?: string) => {
    setData(EMPTY_DATA);
    setCompanyScoutSetupNotice(null);
    setContext({ organizationId, workspaceId: undefined, repositoryId: undefined });
  }, [setContext]);

  const load = useCallback(async () => {
    if (projected !== "ready" || !credentials) {
      clearSensitiveData();
      setServerAccess(projected === "action_needed" ? "action_needed" : "free");
      return;
    }
    if (!specialistNeedsEntitlementVerification(definition.entitlement)) {
      clearSensitiveData();
      setError(null);
      setProjectionWarning(null);
      setServerAccess("ready");
      return;
    }
    setLoading(true);
    setError(null);
    setProjectionWarning(null);
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
        ?? (active === "scout" && orgs.length !== 1 ? undefined : orgs[0]);
      setOrganizations(orgs);
      if (!organization) {
        setData(EMPTY_DATA);
        // Scout owns an explicit company setup/selection flow in this surface.
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
      if (context.organizationId !== organization.id && boundContext?.kind !== active) {
        setContext({ organizationId: organization.id });
      }
      if (active === "scout") {
        const maps = await specialistQuery<CompanyScoutMap[]>(
          credentials, active, "scout_workspaces", organization.id,
        );
        const exactMap = maps.find((item) => item.id === context.workspaceId);
        const map = boundContext?.kind === active
          ? exactMap ?? null
          : companyScoutMap(maps, context.workspaceId);
        if (!map) {
          if (boundContext?.kind === active && context.workspaceId) {
            clearSensitiveData();
            setServerAccess("scope_lost");
            return;
          }
          setData({ ...EMPTY_DATA, companyMap: null });
          return;
        }
        if (!exactMap && boundContext?.kind !== active) {
          setContext({ workspaceId: map.id });
        }
        const [snapshot, changes, simulations] = await Promise.all([
          specialistQuery<{ entries: ScoutSnapshotEntry[] }>(
            credentials, active, "scout_snapshot", organization.id, map.id,
          ),
          specialistQuery<{ changes: ScoutChange[] }>(
            credentials, active, "scout_changes", organization.id, map.id,
          ),
          specialistQuery<ScoutSimulation[]>(
            credentials, active, "scout_simulations", organization.id, map.id,
          ),
        ]);
        setData({
          ...EMPTY_DATA,
          companyMap: map,
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
        const [overviewResult, artifactsResult] = await Promise.allSettled([
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
        if (overviewResult.status === "rejected" && artifactsResult.status === "rejected") {
          throw overviewResult.reason;
        }
        setData({
          ...EMPTY_DATA,
          researchOverview: overviewResult.status === "fulfilled" ? overviewResult.value : null,
          scienceArtifacts: artifactsResult.status === "fulfilled" ? artifactsResult.value : [],
        });
        if (overviewResult.status === "rejected") {
          setProjectionWarning("The research overview is temporarily unavailable. Verified cloud artifacts remain available below.");
        } else if (artifactsResult.status === "rejected") {
          setProjectionWarning("Cloud artifacts are temporarily unavailable. The latest accepted research overview remains visible.");
        }
      } else if (active === "rsi") {
        // RSI state is a typed live object in the conversation timeline. It
        // does not own a parallel dashboard projection or artifact browser.
        setData(EMPTY_DATA);
      } else {
        throw new Error(`No data adapter is registered for specialist ${active}`);
      }
    } catch (cause) {
      setProjectionWarning(null);
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
    definition.entitlement,
    projected,
    setContext,
  ]);

  useEffect(() => {
    setServerAccess("unknown");
    void load();
  }, [load, securityCompletionKey]);

  useEffect(() => {
    if (supportsCanvas) return;
    setCanvasOpen(false);
    setMobilePane("chat");
  }, [supportsCanvas]);

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

  // Company Scout setup is always a visible human action. Session startup never
  // creates cloud authority on the user's behalf; its internal storage id is
  // selected after the user chooses a company.
  const setupCompanyScout = useCallback(async () => {
    if (!credentials || !context.organizationId?.trim()) return;
    const organization = organizations.find((item) => item.id === context.organizationId);
    if (!organization) return;
    setSettingUpCompanyScout(true);
    setCompanyScoutSetupNotice(null);
    setError(null);
    try {
      const created = await specialistSetupCompanyScout(
        credentials,
        context.organizationId,
        organization.name,
      );
      setData((current) => ({
        ...current,
        companyMap: created,
      }));
      setContext({ workspaceId: created.id });
      setCompanyScoutSetupNotice({
        tone: "success",
        message: "Company Scout is ready.",
      });
    } catch (cause) {
      setCompanyScoutSetupNotice({
        tone: "error",
        message: cause instanceof Error ? cause.message : String(cause),
      });
    } finally {
      setSettingUpCompanyScout(false);
    }
  }, [context.organizationId, credentials, organizations, setContext]);

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
      selectOrganization(created.id);
      setOrganizationDialogOpen(false);
      setOrganizationName("");
      setOrganizationDomain("");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setCreatingOrganization(false);
    }
  }, [credentials, organizationDomain, organizationName, selectOrganization]);

  const canvas = (
    <div className="min-h-0 flex-1 overflow-y-auto bg-bg-secondary/30">
      <CanvasStatus loading={(loading || settingUpCompanyScout) && serverAccess === "ready"} error={error} onRetry={() => void load()} />
      {!loading && !error && projectionWarning && (
        <div className="mx-5 mt-4 flex items-start gap-2 border-y border-warning/25 py-3 text-xs leading-5 text-ink-muted">
          <AlertTriangle className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <span>{projectionWarning}</span>
        </div>
      )}
      {!loading && !settingUpCompanyScout && !error && serverAccess === "ready" && (
        active === "scout" ? (
          <ScoutCanvas
            tab={tabs[active] as ScoutTab}
            companyMap={data.companyMap}
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
                workspaceId: data.companyMap?.id,
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
        ) : null
      )}
    </div>
  );

  return (
    <div data-qa={`specialist-workspace-${active}`} className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-bg">
      <header className="flex min-h-16 shrink-0 items-center gap-4 px-5 py-2.5">
        <div className="min-w-0">
          <h1 className="font-serif text-2xl font-semibold tracking-[-0.03em] text-ink">
            {productModule().branding.shortName} {definition.label}
          </h1>
          <p className="mt-0.5 line-clamp-2 max-w-2xl text-xs leading-4 text-ink-muted">{definition.value}</p>
        </div>
        <div className="ml-auto flex items-center gap-2">
          <UpdatePill />
          {supportsCanvas && (
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
          )}
          {organizations.length > 0 && serverAccess === "ready" && (
            <label className="relative hidden md:block">
              <span className="sr-only">{active === "scout" ? "Company" : "Organization"}</span>
              <Building2 className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-ink-faint" />
              <select
                value={active === "scout" ? context.organizationId ?? "" : context.organizationId ?? organizations[0]?.id}
                onChange={(event) => selectOrganization(event.target.value || undefined)}
                disabled={boundContext?.kind === active}
                title={boundContext?.kind === active ? "Start a new specialist conversation to change organization" : undefined}
                className="h-9 appearance-none rounded-xl bg-bg-secondary pl-8 pr-8 text-xs font-medium text-ink-secondary outline-none transition focus:ring-2 focus:ring-accent/20"
              >
                {active === "scout" && <option value="">Choose company…</option>}
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
              aria-label="Create company"
              title="Create company"
              className="grid size-9 place-items-center rounded-xl text-ink-muted transition hover:bg-bg-hover hover:text-ink"
            >
              <Plus className="size-4" />
            </button>
          )}
          {active === "scout" && (
            <CompanyScoutSetupControl
              organizationId={context.organizationId}
              organizations={organizations}
              companyScoutReady={Boolean(data.companyMap)}
              serverReady={serverAccess === "ready"}
              bound={boundContext?.kind === active}
              settingUp={settingUpCompanyScout}
              onSetup={() => void setupCompanyScout()}
            />
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

      {active === "scout" && companyScoutSetupNotice && (
        <CompanyScoutSetupNotice
          notice={companyScoutSetupNotice}
          onDismiss={() => setCompanyScoutSetupNotice(null)}
        />
      )}

      {active === "scout" && scoutScopeOpen && boundContext?.kind !== active && (
        <ScoutScopeDialog
          organizations={organizations}
          companyScoutReady={Boolean(data.companyMap)}
          organizationId={context.organizationId}
          loading={loading}
          settingUpCompanyScout={settingUpCompanyScout}
          onSelectOrganization={selectOrganization}
          onCreateOrganization={() => {
            setScoutScopeOpen(false);
            setOrganizationDialogOpen(true);
          }}
          onSetupCompanyScout={() => void setupCompanyScout()}
          onClose={() => setScoutScopeOpen(false)}
        />
      )}

      {organizationDialogOpen && (
        <ScoutOrganizationDialog
          name={organizationName}
          domain={organizationDomain}
          creating={creatingOrganization}
          onNameChange={setOrganizationName}
          onDomainChange={setOrganizationDomain}
          onCancel={() => setOrganizationDialogOpen(false)}
          onCreate={() => void createOrganization()}
        />
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
            setServerAccess("unknown");
            void productAccess.reload().catch(() => undefined);
          }}
        />
      ) : (
          <div className="flex min-h-0 min-w-0 flex-1 flex-col">
            {supportsCanvas && <div className={cn(
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
            </div>}
            <div className={cn(
              "grid min-h-0 min-w-0 flex-1",
              supportsCanvas && canvasOpen && "xl:grid-cols-[minmax(32rem,1fr)_clamp(22rem,34vw,30rem)]",
            )}>
              <div className={cn("min-h-0 min-w-0", supportsCanvas && mobilePane !== "chat" && "hidden xl:block")}>
                <ContextualConversation kind={active} />
              </div>
              {supportsCanvas && <section
                data-qa={`specialist-canvas-${active}`}
                aria-label={`${definition.label} canvas`}
                className={cn(
                  "flex min-h-0 min-w-0 flex-col bg-bg",
                  mobilePane !== "canvas" && "hidden",
                  canvasOpen && "xl:flex",
                )}
              >
                {canvas}
              </section>}
            </div>
          </div>
      )}
    </div>
  );
}
