import {
  ArrowRight,
  ChartNoAxesColumnIncreasing,
  PlayCircle,
  Route,
  TriangleAlert,
} from "lucide-react";

import type { RsiOverview, ScienceArtifactSegment } from "../../lib/specialistCloud";
import type { RsiTab } from "../../lib/specialists";
import {
  EmptyState,
  MetricCard,
  SectionCard,
  StatusPill,
} from "./SpecialistPrimitives";
import { ScienceArtifactInventory } from "./ScienceArtifactInventory";

export function RsiCanvas({
  tab,
  overview,
  artifacts,
}: {
  tab: RsiTab;
  overview: RsiOverview | null;
  artifacts: ScienceArtifactSegment[];
}) {
  if (!overview && artifacts.length === 0) {
    return (
      <EmptyState
        title="No evaluation research program yet"
        detail="Start an RSI conversation to map the target, build an evaluation world, and search its failure frontier."
      />
    );
  }
  if (!overview) {
    return (
      <CanvasShell title="Cloud RSI artifacts" detail="Verified RSI outputs remain available even while the overview projection is rebuilding.">
        <ScienceArtifactInventory artifacts={artifacts} />
      </CanvasShell>
    );
  }

  if (tab === "worlds") {
    return (
      <CanvasShell title="Evaluation worlds" detail="Pinned environments connect source context to evaluations, runs, and retained counterexamples.">
        <div className="grid gap-3 sm:grid-cols-3">
          <MetricCard label="Worlds" value={overview.worlds.length} />
          <MetricCard label="Evaluations" value={overview.evaluations.length} />
          <MetricCard label="Counterexamples" value={overview.counterexamples.length} tone="warning" />
        </div>
        <SectionCard title="Lineage graph">
          <LineageGraph overview={overview} />
        </SectionCard>
      </CanvasShell>
    );
  }

  if (tab === "evaluations") {
    return (
      <CanvasShell title="Evaluation frontier" detail="Candidates are ranked by information gain, severity, coverage gap, novelty, and cost.">
        <SectionCard title="Evaluation queue">
          <Rows>
            {overview.evaluations.map((evaluation) => (
              <ProjectionRow
                key={evaluation.id}
                icon={<Route className="size-4" />}
                title={evaluation.title}
                detail={`${evaluation.family} · world ${evaluation.worldId}`}
                status={evaluation.status}
                meta={`acquisition ${evaluation.acquisitionScore.toFixed(2)} · severity ${evaluation.severity.toFixed(2)} · gap ${evaluation.coverageGap.toFixed(2)}`}
              />
            ))}
          </Rows>
        </SectionCard>
      </CanvasShell>
    );
  }

  if (tab === "runs") {
    return (
      <CanvasShell title="Evaluation runs" detail="Pinned world revisions, deterministic seeds, driver identities, and terminal receipts.">
        <SectionCard title="Run ledger">
          <Rows>
            {overview.runs.map((run) => (
              <ProjectionRow
                key={run.id}
                icon={<PlayCircle className="size-4" />}
                title={run.evaluationId}
                detail={`${run.driver} · seed ${run.seed}`}
                status={run.status}
                meta={`${run.passed ? "oracle passed" : "oracle failed"} · severity ${run.severity.toFixed(2)}`}
              />
            ))}
          </Rows>
        </SectionCard>
      </CanvasShell>
    );
  }

  if (tab === "frontier") {
    return (
      <CanvasShell title="Failure frontier" detail="Coverage and minimized counterexamples expose the most valuable unknowns.">
        <div className="grid gap-3 sm:grid-cols-3">
          {Object.entries(overview.familyCoverage).map(([family, coverage]) => (
            <MetricCard
              key={family}
              label={family}
              value={`${Math.round(coverage * 100)}%`}
              tone={coverage >= 0.8 ? "good" : coverage < 0.5 ? "warning" : "default"}
            />
          ))}
        </div>
        <SectionCard title="Minimized failures">
          <Rows>
            {overview.counterexamples.map((counterexample) => (
              <ProjectionRow
                key={counterexample.id}
                icon={<TriangleAlert className="size-4" />}
                title={counterexample.title}
                detail={`${counterexample.minimizedPerturbationCount} perturbations after minimization`}
                status={counterexample.reproducible ? "reproducible" : "needs_attention"}
                meta={`severity ${counterexample.severity.toFixed(2)} · evaluation ${counterexample.evaluationId}`}
              />
            ))}
          </Rows>
        </SectionCard>
      </CanvasShell>
    );
  }

  return (
    <CanvasShell title="RSI evidence" detail="Outcomes become evidence only after source, world, driver, seed, evaluator, and provenance validation.">
      <div className="grid gap-3 sm:grid-cols-2">
        <MetricCard label="Evidence envelopes" value={overview.evidenceCount} />
        <MetricCard label="Reproducible failures" value={overview.counterexamples.filter(({ reproducible }) => reproducible).length} tone="warning" />
      </div>
      <SectionCard title="Construction is not truth">
        <div className="flex items-start gap-3 px-4 pb-4 text-xs leading-5 text-ink-muted">
          <ChartNoAxesColumnIncreasing className="mt-0.5 size-4 shrink-0 text-accent" />
          Scout context, memory, world construction, driver execution, evaluator acceptance, and scientific claims remain separate layers.
        </div>
      </SectionCard>
      <ScienceArtifactInventory artifacts={artifacts} />
    </CanvasShell>
  );
}

function LineageGraph({ overview }: { overview: RsiOverview }) {
  const nodes = new Map(overview.lineage.nodes.map((node) => [node.id, node]));
  return (
    <div className="space-y-2 px-4 pb-4">
      {overview.lineage.edges.slice(0, 12).map((edge, index) => {
        const source = nodes.get(edge.source);
        const target = nodes.get(edge.target);
        return (
          <div
            key={`${edge.source}:${edge.target}:${index}`}
            className="grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-2"
          >
            <GraphNode kind={source?.kind ?? "world"} label={source?.label ?? edge.source} />
            <div className="flex items-center gap-1 text-xs font-medium text-ink-faint">
              <span className="hidden sm:inline">{edge.relation.replaceAll("_", " ")}</span>
              <ArrowRight className="size-3.5" />
            </div>
            <GraphNode kind={target?.kind ?? "evaluation"} label={target?.label ?? edge.target} />
          </div>
        );
      })}
      {overview.lineage.edges.length === 0 && (
        <p className="text-xs text-ink-muted">The graph will appear after the first evaluation run.</p>
      )}
    </div>
  );
}

function GraphNode({
  kind,
  label,
}: {
  kind: "world" | "evaluation" | "run" | "counterexample";
  label: string;
}) {
  return (
    <div className="min-w-0 border-l-2 border-border px-3 py-2">
      <div className="text-xs font-semibold uppercase tracking-[0.08em] text-accent">{kind}</div>
      <div className="mt-0.5 truncate text-xs font-medium text-ink-secondary" title={label}>{label}</div>
    </div>
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
        <div className="mt-1 text-xs text-ink-faint">{meta}</div>
      </div>
      <StatusPill status={status} />
    </div>
  );
}
