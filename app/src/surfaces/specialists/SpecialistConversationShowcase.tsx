import { useId, useState } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import {
  Activity,
  ArrowRight,
  CheckCircle2,
  CircleDashed,
  Database,
  FlaskConical,
  GitBranch,
  Network,
  ShieldCheck,
  TriangleAlert,
  Waypoints,
} from "lucide-react";

import {
  specialistConversationPresentation,
  type SpecialistPresentationEvidence,
  type SpecialistPresentationMetric,
  type SpecialistPresentationStage,
  type SpecialistPresentationTone,
} from "../../lib/specialistPresentation";
import type { SpecialistKind } from "../../lib/specialists";
import { cn } from "../../lib/cn";
import {
  DUR,
  EASE,
  RISE_SMALL,
  SLIDE_LEFT,
  SLIDE_RIGHT,
  accessibleMotion,
  staggeredTransition,
} from "../../lib/motion";
import { Mermaid } from "../work/Mermaid";

type PresentationView = "map" | "evidence" | "run";

const VIEWS: ReadonlyArray<{
  id: PresentationView;
  label: string;
  icon: typeof Network;
}> = [
  { id: "map", label: "Map", icon: GitBranch },
  { id: "evidence", label: "Evidence", icon: Database },
  { id: "run", label: "Run", icon: Activity },
];

function toneText(tone: SpecialistPresentationTone): string {
  if (tone === "positive") return "text-success";
  if (tone === "warning") return "text-warning";
  if (tone === "danger") return "text-danger";
  if (tone === "accent") return "text-accent";
  return "text-ink";
}

function toneFill(tone: SpecialistPresentationTone): string {
  if (tone === "positive") return "bg-success";
  if (tone === "warning") return "bg-warning";
  if (tone === "danger") return "bg-danger";
  if (tone === "accent") return "bg-accent";
  return "bg-ink-muted";
}

function SpecialistIcon({ kind }: { kind: SpecialistKind }) {
  const Icon = {
    scout: Network,
    security: ShieldCheck,
    scientist: FlaskConical,
    rsi: Waypoints,
  }[kind] ?? Network;
  return <Icon className="size-4" />;
}

function MetricStrip({
  metrics,
  reduceMotion,
}: {
  metrics: readonly SpecialistPresentationMetric[];
  reduceMotion: boolean;
}) {
  return (
    <div className="grid grid-cols-3 gap-1.5">
      {metrics.map((metric, index) => (
        <m.div
          key={metric.label}
          {...accessibleMotion(RISE_SMALL, reduceMotion)}
          transition={staggeredTransition(reduceMotion, index, 0.045)}
          className="min-w-0 px-1 py-1"
        >
          <div className="truncate text-[0.62rem] font-semibold uppercase tracking-[0.08em] text-ink-faint">
            {metric.label}
          </div>
          <div className={cn("mt-1 truncate text-sm font-semibold", toneText(metric.tone))}>
            {metric.value}
          </div>
          <div className="mt-0.5 truncate text-[0.65rem] text-ink-muted">{metric.detail}</div>
          <div
            role="progressbar"
            aria-label={`${metric.label}: ${metric.value}`}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={metric.progress}
            className="mt-2 h-1 overflow-hidden rounded-full bg-chip"
          >
            <m.div
              className={cn("h-full rounded-full", toneFill(metric.tone))}
              initial={reduceMotion ? { width: `${metric.progress}%` } : { width: 0 }}
              animate={{ width: `${metric.progress}%` }}
              transition={{ duration: reduceMotion ? 0 : DUR.slow, ease: EASE.out }}
            />
          </div>
        </m.div>
      ))}
    </div>
  );
}

function EvidenceRow({
  item,
  index,
  reduceMotion,
}: {
  item: SpecialistPresentationEvidence;
  index: number;
  reduceMotion: boolean;
}) {
  return (
    <m.li
      {...accessibleMotion(RISE_SMALL, reduceMotion)}
      transition={staggeredTransition(reduceMotion, index, 0.05)}
      className="py-2.5"
    >
      <div className="flex items-start gap-2.5">
        <span className={cn("mt-0.5 grid size-6 shrink-0 place-items-center", toneText(item.tone))}>
          {item.tone === "warning" || item.tone === "danger" ? (
            <TriangleAlert className="size-3.5" />
          ) : (
            <CheckCircle2 className="size-3.5" />
          )}
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span className="text-xs font-semibold text-ink">{item.title}</span>
            <span className={cn("text-[0.6rem] font-medium", toneText(item.tone))}>
              {item.status}
            </span>
          </div>
          <p className="mt-1 text-[0.7rem] leading-4 text-ink-muted">{item.detail}</p>
          <div className="mt-2 flex items-center gap-2 text-[0.62rem] text-ink-faint">
            <span className="truncate">{item.source}</span>
            <span aria-hidden>·</span>
            <span className="shrink-0">{item.freshness}</span>
            <span className="ml-auto shrink-0 font-medium text-ink-muted">{item.confidence}%</span>
          </div>
        </div>
      </div>
    </m.li>
  );
}

function StageRow({
  stage,
  index,
  reduceMotion,
}: {
  stage: SpecialistPresentationStage;
  index: number;
  reduceMotion: boolean;
}) {
  const complete = stage.status === "complete";
  const active = stage.status === "active";
  const blocked = stage.status === "blocked";
  return (
    <m.li
      {...accessibleMotion(SLIDE_LEFT, reduceMotion)}
      transition={staggeredTransition(reduceMotion, index, 0.055)}
      className="flex items-start gap-3 py-2.5"
    >
      <span
        className={cn(
          "mt-0.5 grid size-6 shrink-0 place-items-center rounded-full",
          complete && "bg-success/10 text-success",
          active && "bg-accent-soft text-accent",
          blocked && "bg-warning/10 text-warning",
          stage.status === "queued" && "bg-chip text-ink-faint",
        )}
      >
        {complete ? <CheckCircle2 className="size-3.5" /> : <CircleDashed className="size-3.5" />}
      </span>
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-xs font-semibold text-ink">{stage.title}</span>
          <span className={cn(
            "text-[0.6rem] font-medium uppercase tracking-[0.08em]",
            complete && "text-success",
            active && "text-accent",
            blocked && "text-warning",
            stage.status === "queued" && "text-ink-faint",
          )}>
            {stage.status}
          </span>
        </div>
        <p className="mt-1 text-[0.7rem] leading-4 text-ink-muted">{stage.detail}</p>
      </div>
    </m.li>
  );
}

export function SpecialistConversationPresentationCard({
  presentation,
  variant = "conversation",
  onUsePrompt,
}: {
  presentation: NonNullable<ReturnType<typeof specialistConversationPresentation>>;
  variant?: "example" | "conversation";
  onUsePrompt?: (prompt: string) => void;
}) {
  const [view, setView] = useState<PresentationView>("map");
  const reduceMotion = useReducedMotion() ?? false;
  const panelId = useId();

  return (
    <section aria-label={`${presentation.kind} ${variant === "example" ? "example analysis" : "specialist analysis"}`} className="pb-2">
      <div className="mb-3 flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <span className="grid size-7 shrink-0 place-items-center text-accent">
            <SpecialistIcon kind={presentation.kind} />
          </span>
          <div className="min-w-0">
            <div className="text-xs font-semibold text-ink">
              {variant === "example" ? "Illustrative example" : "Specialist analysis"}
            </div>
            <div className="truncate text-[0.65rem] text-ink-faint">
              {variant === "example" ? "Demo data · no work has run" : "Evidence and decision surface"}
            </div>
          </div>
        </div>
        {variant === "example" && onUsePrompt && (
          <button
            type="button"
            onClick={() => onUsePrompt(presentation.prompt)}
            className="flex min-h-8 shrink-0 items-center gap-1 px-1 text-xs font-semibold text-accent transition-colors hover:text-accent-hover focus-visible:outline-none focus-visible:bg-accent-soft"
          >
            Use prompt <ArrowRight className="size-3.5" />
          </button>
        )}
      </div>

      {variant === "example" && (
        <m.div
          {...accessibleMotion(RISE_SMALL, reduceMotion)}
          className="flex justify-end"
        >
          <div className="max-w-[86%] rounded-2xl rounded-br-md bg-bg-tertiary px-3 py-2 text-xs leading-5 text-ink">
            {presentation.prompt}
          </div>
        </m.div>
      )}

      <m.div
        {...accessibleMotion(RISE_SMALL, reduceMotion)}
        transition={staggeredTransition(reduceMotion, 0, 0.04, { delay: 0.08 })}
        className="mt-4 min-w-0"
      >
        <h3 className="font-serif text-lg font-semibold leading-tight tracking-[-0.02em] text-ink">
          {presentation.title}
        </h3>
        <p className="mt-2 text-xs leading-5 text-ink-secondary">{presentation.summary}</p>

        <div className="mt-3">
          <MetricStrip metrics={presentation.metrics} reduceMotion={reduceMotion} />
        </div>

        <div
          role="tablist"
          aria-label="Example analysis views"
          className="mt-3 flex items-center gap-5"
        >
          {VIEWS.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              id={`${panelId}-${id}-tab`}
              type="button"
              role="tab"
              aria-selected={view === id}
              aria-controls={`${panelId}-${id}-panel`}
              onClick={() => setView(id)}
              className={cn(
                "specialist-tab flex min-h-8 items-center justify-center gap-1.5 border-b-2 px-0 text-[0.68rem] font-medium transition-colors",
                view === id
                  ? "border-accent text-accent"
                  : "border-transparent text-ink-muted hover:text-ink",
              )}
            >
              <Icon className="size-3.5" />
              {label}
            </button>
          ))}
        </div>

        <AnimatePresence mode="wait" initial={false}>
          <m.div
            key={view}
            id={`${panelId}-${view}-panel`}
            role="tabpanel"
            aria-labelledby={`${panelId}-${view}-tab`}
            {...accessibleMotion(SLIDE_RIGHT, reduceMotion)}
            transition={staggeredTransition(reduceMotion, 0, 0.04, { duration: DUR.fast })}
            className="mt-2"
          >
            {view === "map" && (
              <div className="py-2">
                <div className="text-[0.65rem] font-semibold uppercase tracking-[0.08em] text-ink-faint">
                  {presentation.diagramTitle}
                </div>
                <Mermaid code={presentation.diagram} />
                <p className="text-[0.7rem] leading-4 text-ink-secondary">
                  <span className="font-semibold text-accent">Decision signal:</span>{" "}
                  {presentation.takeaway}
                </p>
              </div>
            )}
            {view === "evidence" && (
              <ul className="divide-y divide-border-subtle">
                {presentation.evidence.map((item, index) => (
                  <EvidenceRow
                    key={item.id}
                    item={item}
                    index={index}
                    reduceMotion={reduceMotion}
                  />
                ))}
              </ul>
            )}
            {view === "run" && (
              <ol className="divide-y divide-border-subtle">
                {presentation.stages.map((stage, index) => (
                  <StageRow
                    key={stage.id}
                    stage={stage}
                    index={index}
                    reduceMotion={reduceMotion}
                  />
                ))}
              </ol>
            )}
          </m.div>
        </AnimatePresence>

        <p className="mt-3 text-[0.65rem] leading-4 text-ink-faint">{presentation.limitation}</p>
      </m.div>
    </section>
  );
}

export function SpecialistConversationShowcase({
  kind,
  onUsePrompt,
}: {
  kind: SpecialistKind;
  onUsePrompt: (prompt: string) => void;
}) {
  const presentation = specialistConversationPresentation(kind);
  if (!presentation) return null;
  return (
    <SpecialistConversationPresentationCard
      presentation={presentation}
      variant="example"
      onUsePrompt={onUsePrompt}
    />
  );
}
