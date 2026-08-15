import { Activity, Box, Clock3, Database, FileCheck2, GitCompareArrows, Layers3, PlayCircle } from "lucide-react";
import type {
  ScoutChange,
  ScoutSimulation,
  ScoutSnapshotEntry,
  ScoutWorkspace,
} from "../../lib/specialistCloud";
import type { ScoutTab } from "../../lib/specialists";
import { EmptyState, MetricCard, SectionCard, StatusPill } from "./SpecialistPrimitives";

function relativeTime(timestamp: number): string {
  const minutes = Math.max(1, Math.round((Date.now() - timestamp) / 60_000));
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.round(hours / 24)}d ago`;
}

export function scoutSequenceLabel(sequence: number): string {
  return sequence > 0 ? `#${sequence}` : "None yet";
}

export function scoutLoadedCutLabel(workspace: ScoutWorkspace): string {
  const age = relativeTime(workspace.updated_at_ms);
  return workspace.latest_change_sequence > 0
    ? `loaded evidence cut #${workspace.latest_change_sequence} · workspace updated ${age}`
    : `no accepted evidence changes · workspace updated ${age}`;
}

function entryName(entry: ScoutSnapshotEntry): string {
  const value = entry.event.fact.attributes.name;
  return typeof value === "string" && value.trim() ? value : entry.object_id;
}

export function ScoutCanvas({
  tab,
  workspace,
  entries,
  changes,
  simulations,
  onStartSimulation,
  onSelectEntry,
}: {
  tab: ScoutTab;
  workspace: ScoutWorkspace | null;
  entries: ScoutSnapshotEntry[];
  changes: ScoutChange[];
  simulations: ScoutSimulation[];
  onStartSimulation: () => void;
  onSelectEntry: (entry: ScoutSnapshotEntry) => void;
}) {
  if (!workspace) {
    return (
      <EmptyState
        title="Choose or create a Scout workspace"
        detail="A workspace is the explicit cartography boundary. Scout will not infer one from the open folder and will not create one when a conversation starts."
      />
    );
  }

  if (tab === "changes") {
    return (
      <div className="space-y-4 p-5">
        <SectionCard title="Observed changes" detail="Append-only updates from verified sources">
          {changes.length === 0 ? (
            <EmptyState title="No changes in this view" detail="Scout will place accepted source batches and simulations here." />
          ) : (
            <div className="divide-y divide-border-subtle">
              {changes.map((change) => (
                <div key={change.sequence} className="flex items-start gap-3 px-4 py-3">
                  <span className="mt-0.5 grid size-8 shrink-0 place-items-center rounded-lg bg-accent-subtle text-accent">
                    <GitCompareArrows className="size-4" />
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="text-sm font-medium text-ink-secondary">
                      {change.event_type === "batch_accepted" ? "Source evidence accepted" : "Simulation published"}
                    </div>
                    <div className="mt-0.5 truncate text-xs text-ink-muted">
                      {Object.entries(change.payload).map(([key, value]) => `${key}: ${String(value)}`).join(" · ")}
                    </div>
                  </div>
                  <span className="shrink-0 text-xs tabular-nums text-ink-faint">
                    #{change.sequence} · {relativeTime(change.occurred_at_ms)}
                  </span>
                </div>
              ))}
            </div>
          )}
        </SectionCard>
      </div>
    );
  }

  if (tab === "simulations") {
    return (
      <div className="space-y-4 p-5">
        <SectionCard
          title="Impact simulations"
          detail="Snapshot-bound scenarios with explicit evidence coverage"
          action={
            <button
              type="button"
              onClick={onStartSimulation}
              className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-on-accent transition hover:bg-accent/90"
            >
              <PlayCircle className="size-3.5" /> New simulation
            </button>
          }
        >
          <div className="divide-y divide-border-subtle">
            {simulations.map((simulation) => (
              <div key={simulation.id} className="grid gap-2 px-4 py-3 sm:grid-cols-[1fr_auto_auto] sm:items-center">
                <div>
                  <div className="text-sm font-medium text-ink-secondary">{simulation.name}</div>
                  <div className="mt-0.5 text-xs text-ink-muted">
                    Version {simulation.version} · {simulation.membership_count} mapped objects
                  </div>
                </div>
                <StatusPill status={simulation.status} />
                <span className="text-xs text-ink-faint">{relativeTime(simulation.created_at_ms)}</span>
              </div>
            ))}
          </div>
        </SectionCard>
      </div>
    );
  }

  if (tab === "runs") {
    return (
      <div className="space-y-4 p-5">
        <div className="grid gap-3 sm:grid-cols-3">
          <MetricCard label="Evidence runs" value={workspace.run_count} detail="All time" />
          <MetricCard label="Connected sources" value={workspace.source_count} detail="Actively observed" tone="good" />
          <MetricCard label="Collectors online" value={workspace.active_machine_count} detail="Healthy machines" tone="good" />
        </div>
        <SectionCard title="Latest recorded activity" detail="Existing receipts only; opening this view does not refresh sources">
          <div className="flex items-center gap-4 px-4 py-4">
            <span className="grid size-10 place-items-center rounded-xl bg-success/10 text-success">
              <Activity className="size-5" />
            </span>
            <div className="min-w-0 flex-1">
              <div className="text-sm font-medium text-ink">{workspace.display_name}</div>
              <div className="mt-1 text-xs text-ink-muted">
                {workspace.source_count} connected sources · {entries.length} loaded graph objects · updated {relativeTime(workspace.updated_at_ms)}
              </div>
            </div>
            <StatusPill status={workspace.status} />
          </div>
        </SectionCard>
      </div>
    );
  }

  if (tab === "evidence") {
    return (
      <div className="space-y-4 p-5">
        <div className="grid gap-3 sm:grid-cols-3">
          <MetricCard label="Accepted facts" value={entries.length} detail="Current snapshot" tone="good" />
          <MetricCard label="Source connections" value={workspace.source_count} detail="Permission filtered" />
          <MetricCard label="Latest receipt" value={scoutSequenceLabel(workspace.latest_change_sequence)} detail={`Workspace updated ${relativeTime(workspace.updated_at_ms)}`} />
        </div>
        <SectionCard title="Evidence ledger" detail="Accepted observations retain classification and time">
          <div className="divide-y divide-border-subtle">
            {entries.map((entry) => (
              <div key={entry.object_id} className="grid gap-2 px-4 py-3 sm:grid-cols-[2rem_1fr_auto] sm:items-center">
                <span className="grid size-8 place-items-center rounded-lg bg-success/10 text-success">
                  <FileCheck2 className="size-4" />
                </span>
                <span className="min-w-0">
                  <span className="block truncate text-sm font-medium text-ink-secondary">{entryName(entry)}</span>
                  <span className="mt-0.5 block truncate text-xs text-ink-muted">
                    {entry.object_kind} · {entry.object_id}
                  </span>
                </span>
                <span className="text-xs text-ink-faint">
                  {entry.event.classification} · accepted {relativeTime(entry.accepted_at_ms)}
                </span>
              </div>
            ))}
          </div>
        </SectionCard>
      </div>
    );
  }

  return (
    <div className="space-y-4 p-5">
      <div className="grid gap-3 sm:grid-cols-3">
        <MetricCard label="Mapped objects" value={entries.length} detail="Current evidence window" />
        <MetricCard label="Sources" value={workspace.source_count} detail="Connected control planes" tone="good" />
        <MetricCard label="Latest sequence" value={scoutSequenceLabel(workspace.latest_change_sequence)} detail={`Workspace updated ${relativeTime(workspace.updated_at_ms)}`} />
      </div>
      <SectionCard
        title="Observed system"
        detail="Facts are shown with their evidence classification and acceptance time"
        action={
          <span className="flex items-center gap-1.5 text-xs text-ink-muted">
            <Clock3 className="size-3.5" /> {scoutLoadedCutLabel(workspace)}
          </span>
        }
      >
        <div className="divide-y divide-border-subtle">
          {entries.map((entry) => {
            const Icon = entry.object_kind === "edge"
              ? GitCompareArrows
              : entry.event.fact.attributes.kind === "database"
                ? Database
                : entry.object_kind === "claim"
                  ? Layers3
                  : Box;
            return (
              <button
                key={entry.object_id}
                type="button"
                onClick={() => onSelectEntry(entry)}
                className="grid w-full gap-2 px-4 py-3 text-left transition hover:bg-bg-hover sm:grid-cols-[2rem_1fr_auto] sm:items-center"
              >
                <span className="grid size-8 place-items-center rounded-lg bg-bg-secondary text-ink-muted">
                  <Icon className="size-4" />
                </span>
                <span className="min-w-0">
                  <span className="block truncate text-sm font-medium text-ink-secondary">{entryName(entry)}</span>
                  <span className="mt-0.5 block truncate text-xs text-ink-muted">
                    {entry.object_kind} · {entry.object_id}
                  </span>
                </span>
                <span className="text-xs text-ink-faint">
                  {entry.event.classification} · {relativeTime(entry.accepted_at_ms)}
                </span>
              </button>
            );
          })}
        </div>
      </SectionCard>
    </div>
  );
}
