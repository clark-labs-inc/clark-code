import {
  Beaker,
  BookOpenCheck,
  FlaskConical,
  GitBranch,
  PlayCircle,
} from "lucide-react";

import type {
  ResearchOverview,
  ScienceArtifactSegment,
} from "../../lib/specialistCloud";
import type { ScientistTab } from "../../lib/specialists";
import {
  EmptyState,
  MetricCard,
  SectionCard,
  StatusPill,
} from "./SpecialistPrimitives";
import { ScienceArtifactInventory } from "./ScienceArtifactInventory";

export function ScientistCanvas({
  tab,
  overview,
  artifacts,
}: {
  tab: ScientistTab;
  overview: ResearchOverview | null;
  artifacts: ScienceArtifactSegment[];
}) {
  if (!overview && artifacts.length === 0) {
    return (
      <EmptyState
        title="No research programs yet"
        detail="Start a Scientist conversation to turn an objective into a preregistered program."
      />
    );
  }
  if (!overview) {
    return (
      <CanvasShell title="Cloud science artifacts" detail="Verified Scientist outputs remain available even while the overview projection is rebuilding.">
        <ScienceArtifactInventory artifacts={artifacts} />
      </CanvasShell>
    );
  }

  if (tab === "programs") {
    return (
      <CanvasShell title="Research programs" detail="Durable authority, budgets, campaigns, and scientific memory.">
        <div className="grid gap-3 sm:grid-cols-3">
          <MetricCard label="Programs" value={overview.programs.length} />
          <MetricCard label="Evidence records" value={overview.evidenceCount} />
          <MetricCard label="Supported claims" value={overview.supportedClaimCount} tone="good" />
        </div>
        <SectionCard title="Active programs">
          <Rows>
            {overview.programs.map((program) => (
              <ProjectionRow
                key={program.id}
                icon={<FlaskConical className="size-4" />}
                title={program.title}
                detail={program.objective}
                status={program.status}
                meta={`${program.campaignCount} campaigns · ${program.supportedClaimCount} supported claims`}
              />
            ))}
          </Rows>
        </SectionCard>
      </CanvasShell>
    );
  }

  if (tab === "campaigns") {
    return (
      <CanvasShell title="Campaigns" detail="Frozen objectives, preregistered studies, and unresolved authority gates.">
        <SectionCard title="Campaign ledger">
          <Rows>
            {overview.campaigns.map((campaign) => (
              <ProjectionRow
                key={campaign.id}
                icon={<GitBranch className="size-4" />}
                title={campaign.title}
                detail={`${campaign.studyCount} studies · ${campaign.experimentCount} experiments`}
                status={campaign.status}
                meta={`${campaign.unresolvedGateCount} unresolved gates`}
              />
            ))}
          </Rows>
        </SectionCard>
      </CanvasShell>
    );
  }

  if (tab === "experiments") {
    return (
      <CanvasShell title="Experiments" detail="Hypotheses advance only through independent evidence and explicit decisions.">
        <SectionCard title="Experiment graph">
          <Rows>
            {overview.experiments.map((experiment) => (
              <ProjectionRow
                key={experiment.id}
                icon={<Beaker className="size-4" />}
                title={experiment.hypothesis}
                detail={`${experiment.replicationCount} replications · ${experiment.evidenceCount} evidence records`}
                status={experiment.status}
                meta={experiment.decision ? `decision: ${experiment.decision}` : "decision pending"}
              />
            ))}
          </Rows>
        </SectionCard>
      </CanvasShell>
    );
  }

  if (tab === "evidence") {
    return (
      <CanvasShell title="Evidence and claims" detail="Observations remain distinct from claims, decisions, and memory projections.">
        <div className="grid gap-3 sm:grid-cols-2">
          <MetricCard label="Evidence envelopes" value={overview.evidenceCount} />
          <MetricCard label="Supported claims" value={overview.supportedClaimCount} tone="good" />
        </div>
        <SectionCard
          title="Evidence discipline"
          detail="Every claim links to immutable provenance, calibration, rights tags, and bitemporal source snapshots."
        >
          <div className="flex items-start gap-3 px-4 pb-4 text-xs leading-5 text-ink-muted">
            <BookOpenCheck className="mt-0.5 size-4 shrink-0 text-accent" />
            Model reasoning may propose a claim, but only evaluator and instrument receipts can support it.
          </div>
        </SectionCard>
        <ScienceArtifactInventory artifacts={artifacts} />
      </CanvasShell>
    );
  }

  return (
    <CanvasShell title="Research runs" detail="Live, terminal, and interrupted execution with exactly-once effect receipts.">
      <SectionCard title="Run ledger">
        <Rows>
          {overview.runs.map((run) => (
            <ProjectionRow
              key={run.id}
              icon={<PlayCircle className="size-4" />}
              title={run.id}
              detail={`${run.effectCount} effects · experiment ${run.experimentId}`}
              status={run.status}
              meta={`${run.interruptedEffectCount} interrupted effects`}
            />
          ))}
        </Rows>
      </SectionCard>
    </CanvasShell>
  );
}

function CanvasShell({
  title,
  detail,
  children,
}: {
  title: string;
  detail: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-4 p-5">
      <div>
        <h2 className="font-serif text-xl font-semibold tracking-[-0.02em] text-ink">{title}</h2>
        <p className="mt-1 text-xs leading-5 text-ink-muted">{detail}</p>
      </div>
      {children}
    </div>
  );
}

function Rows({ children }: { children: React.ReactNode }) {
  return <div className="divide-y divide-border-subtle">{children}</div>;
}

function ProjectionRow({
  icon,
  title,
  detail,
  status,
  meta,
}: {
  icon: React.ReactNode;
  title: string;
  detail: string;
  status: string;
  meta: string;
}) {
  return (
    <div className="flex items-start gap-3 px-4 py-3">
      <span className="mt-0.5 grid size-8 shrink-0 place-items-center rounded-lg bg-accent-soft text-accent">
        {icon}
      </span>
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-ink">{title}</div>
        <div className="mt-0.5 text-xs leading-5 text-ink-muted">{detail}</div>
        <div className="mt-1 text-[0.68rem] text-ink-faint">{meta}</div>
      </div>
      <StatusPill status={status} />
    </div>
  );
}
