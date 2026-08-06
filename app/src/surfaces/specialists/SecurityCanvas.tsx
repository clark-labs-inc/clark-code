import { useState } from "react";
import { AlertOctagon, Beaker, Download, GitBranch, Radar, ScanSearch, ShieldCheck } from "lucide-react";
import type {
  SecurityCampaign,
  SecurityFinding,
  SecurityPosture,
  SecurityRepository,
  SecurityScan,
} from "../../lib/specialistCloud";
import type { SecurityTab } from "../../lib/specialists";
import { cn } from "../../lib/cn";
import { EmptyState, MetricCard, SectionCard, StatusPill } from "./SpecialistPrimitives";

type PostureFilter = "repositories" | "critical" | "high";

function relativeDate(value: string | null | undefined): string {
  if (!value) return "Never";
  const hours = Math.max(1, Math.round((Date.now() - Date.parse(value)) / 3_600_000));
  return hours < 24 ? `${hours}h ago` : `${Math.round(hours / 24)}d ago`;
}

function SeverityPill({ severity }: { severity: SecurityFinding["currentSeverity"] }) {
  return (
    <span className={cn(
      "rounded-full px-2 py-0.5 text-[0.68rem] font-semibold capitalize",
      severity === "critical" && "bg-danger/10 text-danger",
      severity === "high" && "bg-warning/10 text-warning",
      severity === "medium" && "bg-accent-soft text-accent",
      (severity === "low" || severity === "informational") && "bg-chip text-ink-muted",
    )}>
      {severity}
    </span>
  );
}

export function SecurityCanvas({
  tab,
  posture,
  repositories,
  findings,
  candidates,
  scans,
  campaigns,
  onSaveScanPdf,
  onSelectTab,
  onSelectRepository,
  onSelectFinding,
  onStartScan,
  onResearchCandidate,
  onCreateCampaign,
}: {
  tab: SecurityTab;
  posture: SecurityPosture | null;
  repositories: SecurityRepository[];
  findings: SecurityFinding[];
  candidates: SecurityFinding[];
  scans: SecurityScan[];
  campaigns: SecurityCampaign[];
  onSaveScanPdf: (scan: SecurityScan) => Promise<boolean>;
  onSelectTab: (tab: SecurityTab) => void;
  onSelectRepository: (repository: SecurityRepository) => void;
  onSelectFinding: (finding: SecurityFinding) => void;
  onStartScan: () => void;
  onResearchCandidate: () => void;
  onCreateCampaign: (title: string, description: string, findingIds: string[]) => Promise<void>;
}) {
  const [postureFilter, setPostureFilter] = useState<PostureFilter>("repositories");
  const [campaignFormOpen, setCampaignFormOpen] = useState(false);
  const [campaignTitle, setCampaignTitle] = useState("");
  const [campaignDescription, setCampaignDescription] = useState("");
  const [campaignFindingIds, setCampaignFindingIds] = useState<string[]>([]);
  const [creatingCampaign, setCreatingCampaign] = useState(false);
  const [campaignError, setCampaignError] = useState<string | null>(null);
  const [exportingScanId, setExportingScanId] = useState<string | null>(null);
  const [exportError, setExportError] = useState<string | null>(null);

  const saveScanPdf = async (scan: SecurityScan) => {
    setExportingScanId(scan.id);
    setExportError(null);
    try {
      await onSaveScanPdf(scan);
    } catch (cause) {
      setExportError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setExportingScanId(null);
    }
  };

  if (tab === "findings" || tab === "zero-days") {
    const rows = tab === "zero-days"
      ? candidates
      : findings.filter((finding) => finding.workflowState !== "closed");
    return (
      <div className="space-y-4 p-5">
        <SectionCard
          title={tab === "zero-days" ? "Novel vulnerability research" : "Validated findings"}
          detail={tab === "zero-days"
            ? "Candidates stay unconfirmed until controls and prior-art evidence agree"
            : "Stable root-cause identities across scans and repositories"}
          action={
            <button
              type="button"
              onClick={tab === "zero-days" ? onResearchCandidate : onStartScan}
              className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-white transition hover:bg-accent/90"
            >
              {tab === "zero-days" ? <Beaker className="size-3.5" /> : <ScanSearch className="size-3.5" />}
              {tab === "zero-days" ? "Research candidate" : "Start scan"}
            </button>
          }
        >
          {rows.length === 0 ? (
            <EmptyState
              title={tab === "zero-days" ? "No novel candidates" : "No open findings"}
              detail="Clark only clears a surface after sufficient coverage; unknown coverage remains explicit."
            />
          ) : (
            <div className="divide-y divide-border-subtle">
              {rows.map((finding) => (
                <button
                  key={finding.id}
                  type="button"
                  onClick={() => onSelectFinding(finding)}
                  className="grid w-full gap-2 px-4 py-3 text-left transition hover:bg-bg-hover sm:grid-cols-[1fr_auto_auto] sm:items-center"
                >
                  <span className="min-w-0">
                    <span className="block truncate text-sm font-medium text-ink-secondary">{finding.title}</span>
                    <span className="mt-0.5 block truncate text-xs text-ink-muted">
                      {finding.category} · {finding.validationState.replaceAll("_", " ")}
                    </span>
                  </span>
                  <SeverityPill severity={finding.currentSeverity} />
                  <span className="text-xs text-ink-faint">{relativeDate(finding.lastSeenAt)}</span>
                </button>
              ))}
            </div>
          )}
        </SectionCard>
      </div>
    );
  }

  if (tab === "campaigns") {
    return (
      <div className="space-y-4 p-5">
        <SectionCard
          title="Remediation campaigns"
          detail="Coordinate related fixes without losing finding-level evidence"
          action={
            <button
              type="button"
              onClick={() => {
                setCampaignFormOpen((open) => !open);
                setCampaignError(null);
              }}
              className="rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-white transition hover:bg-accent/90"
            >
              New campaign
            </button>
          }
        >
          {campaignFormOpen && (
            <form
              className="space-y-3 border-b border-border-subtle p-4"
              onSubmit={(event) => {
                event.preventDefault();
                setCreatingCampaign(true);
                setCampaignError(null);
                void onCreateCampaign(
                  campaignTitle.trim(),
                  campaignDescription.trim(),
                  campaignFindingIds,
                ).then(() => {
                  setCampaignFormOpen(false);
                  setCampaignTitle("");
                  setCampaignDescription("");
                  setCampaignFindingIds([]);
                }).catch((cause: unknown) => {
                  setCampaignError(cause instanceof Error ? cause.message : String(cause));
                }).finally(() => setCreatingCampaign(false));
              }}
            >
              <div className="grid gap-3 sm:grid-cols-2">
                <label className="space-y-1 text-xs font-medium text-ink-muted">
                  Campaign title
                  <input
                    required
                    value={campaignTitle}
                    onChange={(event) => setCampaignTitle(event.target.value)}
                    className="w-full rounded-lg border border-border bg-bg px-3 py-2 text-sm text-ink-secondary outline-none focus:border-accent"
                  />
                </label>
                <label className="space-y-1 text-xs font-medium text-ink-muted">
                  Description
                  <input
                    required
                    value={campaignDescription}
                    onChange={(event) => setCampaignDescription(event.target.value)}
                    className="w-full rounded-lg border border-border bg-bg px-3 py-2 text-sm text-ink-secondary outline-none focus:border-accent"
                  />
                </label>
              </div>
              <fieldset className="space-y-2">
                <legend className="text-xs font-medium text-ink-muted">Open findings</legend>
                {findings.filter((finding) => finding.workflowState !== "closed").map((finding) => (
                  <label key={finding.id} className="flex items-center gap-2 text-xs text-ink-secondary">
                    <input
                      type="checkbox"
                      checked={campaignFindingIds.includes(finding.id)}
                      onChange={(event) => setCampaignFindingIds((selected) => event.target.checked
                        ? [...selected, finding.id]
                        : selected.filter((id) => id !== finding.id))}
                    />
                    <SeverityPill severity={finding.currentSeverity} />
                    <span>{finding.title}</span>
                  </label>
                ))}
              </fieldset>
              {campaignError && <p className="text-xs text-danger">{campaignError}</p>}
              <button
                type="submit"
                disabled={creatingCampaign || campaignFindingIds.length === 0}
                className="rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-white disabled:cursor-not-allowed disabled:opacity-50"
              >
                {creatingCampaign ? "Creating…" : "Create campaign"}
              </button>
            </form>
          )}
          {campaigns.length === 0 ? (
            <EmptyState title="No remediation campaigns" detail="Create a campaign from one or more open findings." />
          ) : (
            <div className="grid gap-3 p-4 lg:grid-cols-3">
            {campaigns.map((campaign) => (
              <div key={campaign.id} className="border-t border-border-subtle px-3 py-3 text-left">
                <div className="text-sm font-medium text-ink-secondary">{campaign.title}</div>
                <div className="mt-1 line-clamp-2 text-xs text-ink-muted">{campaign.description}</div>
                <div className="mt-3 flex items-center justify-between text-xs text-ink-muted">
                  <span>{campaign.verifiedFindingCount}/{campaign.findingCount} verified</span>
                  <StatusPill status={campaign.status} />
                </div>
              </div>
            ))}
            </div>
          )}
        </SectionCard>
      </div>
    );
  }

  if (tab === "scans") {
    return (
      <div className="space-y-4 p-5">
        <SectionCard title="Scan history" detail="Latest sealed and in-progress repository scans">
          {exportError && (
            <div role="alert" className="border-b border-danger/20 bg-danger/5 px-4 py-2 text-xs text-danger">
              Could not save the PDF: {exportError}
            </div>
          )}
          {scans.length === 0 ? (
            <EmptyState title="No scan runs yet" detail="Start a standard, diff, or deep scan from the contextual chat." />
          ) : (
            <div className="divide-y divide-border-subtle">
              {scans.map((scan) => (
                <div key={scan.id} className="grid gap-2 px-4 py-3 sm:grid-cols-[1fr_auto_7rem_5rem_7.5rem] sm:items-center">
                  <div>
                    <div className="text-sm font-medium capitalize text-ink-secondary">{scan.mode} scan</div>
                    <div className="mt-0.5 text-xs text-ink-muted">{scan.repositoryId}</div>
                  </div>
                  <StatusPill status={scan.status} />
                  <span className="truncate text-xs text-ink-muted">Clark Security</span>
                  <span className="text-xs text-ink-faint">{relativeDate(scan.createdAt)}</span>
                  <button
                    type="button"
                    onClick={() => void saveScanPdf(scan)}
                    disabled={exportingScanId !== null}
                    className="flex items-center justify-center gap-1.5 rounded-lg border border-border px-2.5 py-1.5 text-xs font-semibold text-ink-secondary transition hover:bg-bg-hover disabled:cursor-wait disabled:opacity-60"
                    aria-label={`Save ${scan.mode} scan as PDF`}
                  >
                    <Download className="size-3.5" />
                    {exportingScanId === scan.id ? "Saving..." : "Save as PDF"}
                  </button>
                </div>
              ))}
            </div>
          )}
        </SectionCard>
      </div>
    );
  }

  const visibleRepositories = repositories.filter((repository) => (
    postureFilter === "repositories"
      || (postureFilter === "critical" && repository.openCriticalCount > 0)
      || (postureFilter === "high" && repository.openHighCount > 0)
  ));
  const visibleFindings = findings.filter((finding) => (
    finding.workflowState !== "closed"
      && (postureFilter === "repositories" || finding.currentSeverity === postureFilter)
  ));

  return (
    <div className="space-y-4 p-5">
      <div className="grid gap-3 sm:grid-cols-4">
        <button
          type="button"
          aria-pressed={postureFilter === "repositories"}
          onClick={() => setPostureFilter("repositories")}
          className="rounded-lg text-left outline-none transition hover:bg-bg-hover focus-visible:ring-2 focus-visible:ring-accent/30"
        >
          <MetricCard label="Repositories" value={`${posture?.scannedRepositoryCount ?? 0}/${posture?.repositoryCount ?? 0}`} detail="Sufficiently scanned" tone="good" />
        </button>
        <button
          type="button"
          aria-pressed={postureFilter === "critical"}
          onClick={() => setPostureFilter("critical")}
          className="rounded-lg text-left outline-none transition hover:bg-bg-hover focus-visible:ring-2 focus-visible:ring-accent/30"
        >
          <MetricCard label="Critical" value={posture?.openCriticalCount ?? 0} detail="Open validated" tone={(posture?.openCriticalCount ?? 0) > 0 ? "danger" : "good"} />
        </button>
        <button
          type="button"
          aria-pressed={postureFilter === "high"}
          onClick={() => setPostureFilter("high")}
          className="rounded-lg text-left outline-none transition hover:bg-bg-hover focus-visible:ring-2 focus-visible:ring-accent/30"
        >
          <MetricCard label="High" value={posture?.openHighCount ?? 0} detail="Open validated" tone={(posture?.openHighCount ?? 0) > 0 ? "warning" : "good"} />
        </button>
        <button
          type="button"
          onClick={() => onSelectTab("zero-days")}
          className="rounded-lg text-left outline-none transition hover:bg-bg-hover focus-visible:ring-2 focus-visible:ring-accent/30"
        >
          <MetricCard label="Novel" value={posture?.confirmedNovelCount ?? 0} detail={`${posture?.suspectedNovelCount ?? 0} under research`} />
        </button>
      </div>
      <SectionCard
        title={postureFilter === "repositories" ? "Repository posture" : `${postureFilter === "critical" ? "Critical" : "High"} repositories`}
        detail="Coverage, current risk, and the latest sealed scan"
      >
        <div className="divide-y divide-border-subtle">
          {visibleRepositories.length === 0 ? (
            <EmptyState title={`No ${postureFilter} repositories`} detail="No repositories match this posture filter." />
          ) : visibleRepositories.map((repository) => (
            <button
              key={repository.repositoryId}
              type="button"
              onClick={() => onSelectRepository(repository)}
              className="grid w-full gap-2 px-4 py-3 text-left transition hover:bg-bg-hover sm:grid-cols-[2rem_1fr_auto_5rem] sm:items-center"
            >
              <span className={cn(
                "grid size-8 place-items-center rounded-lg",
                repository.stale ? "bg-warning/10 text-warning" : "bg-success/10 text-success",
              )}>
                {repository.stale ? <Radar className="size-4" /> : <ShieldCheck className="size-4" />}
              </span>
              <span className="min-w-0">
                <span className="block truncate text-sm font-medium text-ink-secondary">
                  {repository.serviceName ?? repository.canonicalRemote ?? repository.repositoryId}
                </span>
                <span className="mt-0.5 flex items-center gap-1 truncate text-xs text-ink-muted">
                  <GitBranch className="size-3" />
                  {repository.canonicalRemote ?? repository.repositoryId} · {relativeDate(repository.latestScanCreatedAt)}
                </span>
              </span>
              <span className="flex items-center gap-2">
                {repository.openCriticalCount > 0 && (
                  <span className="flex items-center gap-1 text-xs font-medium text-danger">
                    <AlertOctagon className="size-3.5" /> {repository.openCriticalCount} critical
                  </span>
                )}
                <StatusPill status={repository.stale ? "stale" : repository.latestScanStatus ?? "unknown"} />
              </span>
              <span className="text-right text-sm font-semibold tabular-nums text-ink">
                {repository.riskScore}
                <span className="ml-1 text-[0.65rem] font-normal text-ink-faint">risk</span>
              </span>
            </button>
          ))}
        </div>
      </SectionCard>
      <SectionCard title="Needs attention" detail="Highest-confidence open work across this organization">
        <div className="divide-y divide-border-subtle">
          {visibleFindings.length === 0 ? (
            <EmptyState title="No matching findings" detail="No open findings match this posture filter." />
          ) : visibleFindings.slice(0, 3).map((finding) => (
              <button
                key={finding.id}
                type="button"
                onClick={() => onSelectFinding(finding)}
                className="grid w-full gap-2 px-4 py-3 text-left transition hover:bg-bg-hover sm:grid-cols-[1fr_auto_auto] sm:items-center"
              >
                <span className="min-w-0">
                  <span className="block truncate text-sm font-medium text-ink-secondary">{finding.title}</span>
                  <span className="mt-0.5 block truncate text-xs text-ink-muted">
                    {finding.category} · {finding.validationState.replaceAll("_", " ")}
                  </span>
                </span>
                <SeverityPill severity={finding.currentSeverity} />
                <span className="text-xs text-ink-faint">{relativeDate(finding.lastSeenAt)}</span>
              </button>
            ))}
        </div>
      </SectionCard>
    </div>
  );
}
