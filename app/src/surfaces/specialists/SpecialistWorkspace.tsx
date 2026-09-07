import { useCallback, useEffect, useState } from "react";
import {
  AlertTriangle,
  Building2,
  ChevronDown,
  MessageSquareText,
  Moon,
  PanelRightClose,
  PanelRightOpen,
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
  type SecurityTab,
  type ScientistTab,
} from "../../lib/specialists";
import {
  specialistEntitlement,
  specialistOrganizations,
  specialistQuery,
  specialistCreateSecurityCampaign,
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
import { SecurityCanvas } from "./SecurityCanvas";
import { ScientistCanvas } from "./ScientistCanvas";
import { CanvasStatus } from "./SpecialistPrimitives";
import { SpecialistAccessGate } from "./SpecialistAccessGate";
import { ContextualConversation } from "./ContextualConversation";

interface SpecialistData {
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
  const active = useSpecialistStore((state) => state.active) ?? "security";
  const tabs = useSpecialistStore((state) => state.tabs);
  const contexts = useSpecialistStore((state) => state.contexts);
  const setTab = useSpecialistStore((state) => state.setTab);
  const setContext = useSpecialistStore((state) => state.setContext);
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
  const definition = SPECIALISTS[active];
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
        ?? orgs[0];
      setOrganizations(orgs);
      if (!organization) {
        setData(EMPTY_DATA);
        setServerAccess(orgs.length === 0 ? "organization_required" : "ready");
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
      if (active === "security") {
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

  const canvas = (
    <div className="min-h-0 flex-1 overflow-y-auto bg-bg-secondary/30">
      <CanvasStatus loading={loading && serverAccess === "ready"} error={error} onRetry={() => void load()} />
      {!loading && !error && projectionWarning && (
        <div className="mx-5 mt-4 flex items-start gap-2 border-y border-warning/25 py-3 text-xs leading-5 text-ink-muted">
          <AlertTriangle className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <span>{projectionWarning}</span>
        </div>
      )}
      {!loading && !error && serverAccess === "ready" && (
        active === "security" ? (
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
                value={context.organizationId ?? organizations[0]?.id}
                onChange={(event) => selectOrganization(event.target.value || undefined)}
                disabled={boundContext?.kind === active}
                title={boundContext?.kind === active ? "Start a new specialist conversation to change organization" : undefined}
                className="h-9 appearance-none rounded-xl bg-bg-secondary pl-8 pr-8 text-xs font-medium text-ink-secondary outline-none transition focus:ring-2 focus:ring-accent/20"
              >
                {organizations.map((organization) => (
                  <option key={organization.id} value={organization.id}>{organization.name}</option>
                ))}
              </select>
              <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 size-3.5 -translate-y-1/2 text-ink-faint" />
            </label>
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

      {access !== "ready" ? (
        <SpecialistAccessGate
          key={`${active}:${access}`}
          kind={active}
          state={access}
          onProductAction={() => {
            if (accessCapability?.actionUrl) void openExternal(accessCapability.actionUrl);
            else setSettingsOpen(true);
          }}
          onWorkspaceSetup={() => setSettingsOpen(true)}
          onRetry={() => {
            setServerAccess("unknown");
            void productAccess.reload().catch(() => undefined);
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
