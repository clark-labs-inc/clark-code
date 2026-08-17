import { useEffect, useId, useMemo, useRef, useState, type RefObject } from "react";
import { useReducedMotion } from "motion/react";
import {
  ArrowRight,
  ChartNoAxesColumnIncreasing,
  Check,
  ChevronDown,
  Code2,
  Lightbulb,
  Pause,
  RotateCcw,
  Search,
  ShieldCheck,
} from "lucide-react";

import type {
  SpecialistConversationPresentation,
  SpecialistPresentationMetric,
  SpecialistPresentationStage,
} from "../../lib/specialistPresentation";
import { cn } from "../../lib/cn";
import {
  renderRsiLoop,
  type RsiLoopVisualStatus,
} from "./rsiLoopRenderer";

const LOOP_STAGES = [
  { id: "inspect", label: "Inspect", icon: Search },
  { id: "propose", label: "Propose", icon: Lightbulb },
  { id: "code", label: "Code", icon: Code2 },
  { id: "measure", label: "Measure", icon: ChartNoAxesColumnIncreasing },
  { id: "decide", label: "Decide", icon: ShieldCheck },
] as const;

interface LoopStage {
  id: (typeof LOOP_STAGES)[number]["id"];
  label: string;
  detail: string;
  status: RsiLoopVisualStatus;
  icon: (typeof LOOP_STAGES)[number]["icon"];
}

function stageStatus(stage: SpecialistPresentationStage | undefined): RsiLoopVisualStatus {
  if (!stage) return "queued";
  return stage.status;
}

function stageMatches(stage: SpecialistPresentationStage, id: string): boolean {
  const searchable = `${stage.id} ${stage.title}`.toLowerCase();
  return searchable.includes(id);
}

export function rsiLoopStages(
  presentation: SpecialistConversationPresentation,
): LoopStage[] {
  return LOOP_STAGES.map((definition, index) => {
    const source = presentation.stages.find((stage) => stageMatches(stage, definition.id))
      ?? presentation.stages[index];
    return {
      ...definition,
      detail: source?.detail ?? "Waiting for this step.",
      status: stageStatus(source),
    };
  });
}

function metric(
  presentation: SpecialistConversationPresentation,
  ...labels: string[]
): SpecialistPresentationMetric | undefined {
  return presentation.metrics.find((candidate) => {
    const normalized = candidate.label.toLowerCase();
    return labels.some((label) => normalized.includes(label));
  });
}

function iterationNumber(presentation: SpecialistConversationPresentation): number {
  const values = [
    presentation.title,
    presentation.summary,
    presentation.takeaway,
    ...presentation.stages.flatMap((stage) => [stage.title, stage.detail]),
  ];
  for (const value of values) {
    const match = value.match(/iteration\s+(\d+)/i);
    if (match) return Number(match[1]);
  }
  return 1;
}

function currentStageIndex(stages: readonly LoopStage[]): number {
  const active = stages.findIndex((stage) =>
    stage.status === "active" || stage.status === "blocked");
  if (active >= 0) return active;
  const queued = stages.findIndex((stage) => stage.status === "queued");
  return queued >= 0 ? queued : Math.max(0, stages.length - 1);
}

function useLoopCanvas(
  stages: readonly LoopStage[],
  activeIndex: number,
): {
  canvasRef: RefObject<HTMLCanvasElement | null>;
  containerRef: RefObject<HTMLDivElement | null>;
  ready: boolean;
} {
  const reduceMotion = useReducedMotion() ?? false;
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [ready, setReady] = useState(false);
  const statuses = stages.map((stage) => stage.status).join(":");

  useEffect(() => {
    const canvas = canvasRef.current;
    const container = containerRef.current;
    if (!canvas || !container) return;

    let cancelled = false;
    let visible = true;
    let drawing = false;
    let frame = 0;
    let lastFrame = 0;
    const animated = !reduceMotion && stages[activeIndex]?.status === "active";

    const draw = async (time: number) => {
      if (cancelled || drawing || !visible || document.hidden) return;
      drawing = true;
      const styles = getComputedStyle(container);
      const rendered = await renderRsiLoop(
        canvas,
        canvas.getBoundingClientRect().width || 176,
        window.devicePixelRatio || 1,
        {
          stages: stages.map((stage) => stage.status),
          activeIndex,
          phase: time / 420,
          colors: {
            accent: styles.getPropertyValue("--color-accent").trim() || "#9b8cff",
            complete: styles.getPropertyValue("--color-info").trim() || "#6fa8bd",
            warning: styles.getPropertyValue("--color-warning").trim() || "#d0a24a",
            muted: styles.getPropertyValue("--color-ink-faint").trim() || "#918e99",
          },
        },
      );
      drawing = false;
      if (!cancelled) setReady(rendered);
    };

    const tick = (time: number) => {
      if (cancelled) return;
      if (time - lastFrame >= 50) {
        lastFrame = time;
        void draw(time);
      }
      if (animated) frame = requestAnimationFrame(tick);
    };

    const start = () => {
      void draw(performance.now());
      if (animated && !frame) frame = requestAnimationFrame(tick);
    };
    const stop = () => {
      if (frame) cancelAnimationFrame(frame);
      frame = 0;
    };

    const intersection = typeof IntersectionObserver === "undefined"
      ? null
      : new IntersectionObserver(([entry]) => {
        visible = entry?.isIntersecting ?? true;
        if (visible) start();
        else stop();
      }, { rootMargin: "80px" });
    intersection?.observe(container);

    const resize = typeof ResizeObserver === "undefined"
      ? null
      : new ResizeObserver(() => void draw(performance.now()));
    resize?.observe(canvas);

    const theme = typeof MutationObserver === "undefined"
      ? null
      : new MutationObserver(() => void draw(performance.now()));
    theme?.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class", "data-interface-contrast"],
    });

    const onVisibilityChange = () => {
      if (document.hidden) stop();
      else if (visible) start();
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    start();

    return () => {
      cancelled = true;
      stop();
      intersection?.disconnect();
      resize?.disconnect();
      theme?.disconnect();
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, [activeIndex, reduceMotion, statuses]);

  return { canvasRef, containerRef, ready };
}

function LoopGraphic({ stages, iteration }: { stages: readonly LoopStage[]; iteration: number }) {
  const activeIndex = currentStageIndex(stages);
  const { canvasRef, containerRef, ready } = useLoopCanvas(stages, activeIndex);

  return (
    <div ref={containerRef} className="relative mx-auto h-56 w-64 shrink-0" data-qa="rsi-loop-graphic">
      <canvas
        ref={canvasRef}
        aria-hidden="true"
        className={cn(
          "absolute left-1/2 top-1/2 size-48 -translate-x-1/2 -translate-y-1/2 transition-opacity",
          ready ? "opacity-100" : "opacity-0",
        )}
      />
      {ready ? (
        <>
          <div className="pointer-events-none absolute inset-0">
            {stages.map((stage, index) => {
              const angle = -90 + index * 72;
              const left = 50 + Math.cos((angle * Math.PI) / 180) * 38;
              const top = 50 + Math.sin((angle * Math.PI) / 180) * 38;
              const Icon = stage.icon;
              return (
                <div
                  key={stage.id}
                  title={`${stage.label}: ${stage.status}`}
                  className="absolute flex -translate-x-1/2 -translate-y-1/2 flex-col items-center gap-1"
                  style={{ left: `${left}%`, top: `${top}%` }}
                >
                  <span className={cn(
                    "grid size-8 place-items-center rounded-full border bg-bg-elevated",
                    stage.status === "complete" && "border-info/60 text-info",
                    stage.status === "active" && "border-accent text-accent",
                    stage.status === "blocked" && "border-warning text-warning",
                    stage.status === "queued" && "border-border-strong text-ink-faint",
                  )}>
                    <Icon className="size-4" />
                  </span>
                  <span className={cn(
                    "rounded bg-bg/85 px-1 text-xs font-medium",
                    stage.status === "active" ? "text-accent" : "text-ink-muted",
                  )}>
                    {stage.label}
                  </span>
                </div>
              );
            })}
          </div>
          <div className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 text-center">
            <div className="font-serif text-3xl font-semibold text-ink">{iteration}</div>
            <div className="text-xs text-ink-muted">iteration</div>
          </div>
        </>
      ) : (
        <ol className="flex h-full flex-col justify-center gap-1.5" aria-label="RSI loop stages">
          {stages.map((stage) => (
            <li key={stage.id} className="flex items-center gap-2 text-xs text-ink-muted">
              <span className={cn(
                "grid size-5 place-items-center rounded-full border",
                stage.status === "complete" && "border-info/60 text-info",
                stage.status === "active" && "border-accent text-accent",
                stage.status === "blocked" && "border-warning text-warning",
                stage.status === "queued" && "border-border text-ink-faint",
              )}>
                {stage.status === "complete" ? <Check className="size-3" /> : null}
              </span>
              {stage.label}
            </li>
          ))}
        </ol>
      )}
      <span className="sr-only">Iteration {iteration}. Current stage: {stages[activeIndex]?.label}.</span>
    </div>
  );
}

export function RsiLoopPulseCard({
  presentation,
  variant = "conversation",
  onUsePrompt,
  onPause,
}: {
  presentation: SpecialistConversationPresentation;
  variant?: "example" | "conversation";
  onUsePrompt?: (prompt: string) => void;
  onPause?: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const detailsId = useId();
  const stages = useMemo(() => rsiLoopStages(presentation), [presentation]);
  const activeIndex = currentStageIndex(stages);
  const current = stages[activeIndex];
  const best = metric(presentation, "best safe", "best result", "best");
  const kept = metric(presentation, "kept", "accepted");
  const undone = metric(presentation, "undone", "rollback");
  const guardrailFailure = presentation.evidence.some((item) =>
    item.tone === "danger" || item.tone === "warning");
  const production = presentation.evidence.find((item) =>
    item.title.toLowerCase().includes("production"));
  const canPause = Boolean(onPause && current?.status === "active");

  return (
    <section aria-label="RSI recursive improvement loop" className="pb-2" data-qa="rsi-loop-pulse">
      {variant === "example" && (
        <div className="mb-3 flex items-center justify-between gap-3">
          <div>
            <div className="text-xs font-semibold text-ink">Illustrative example</div>
            <div className="text-xs text-ink-faint">Demo data · no code has changed</div>
          </div>
          {onUsePrompt && (
            <button
              type="button"
              onClick={() => onUsePrompt(presentation.prompt)}
              className="flex min-h-8 items-center gap-1 text-xs font-semibold text-accent transition-colors hover:text-accent-hover focus-visible:outline-none focus-visible:bg-accent-soft"
            >
              Use prompt <ArrowRight className="size-3.5" />
            </button>
          )}
        </div>
      )}

      <div className="overflow-hidden rounded-2xl border border-border bg-bg-elevated">
        <div className="flex flex-col gap-4 p-4 sm:flex-row sm:items-center sm:gap-5">
          <LoopGraphic stages={stages} iteration={iterationNumber(presentation)} />

          <div className="min-w-0 flex-1">
            <div className="flex items-start gap-3">
              <span className={cn(
                "mt-1 size-2.5 shrink-0 rounded-full",
                current?.status === "blocked" ? "bg-warning" : "bg-accent",
              )} aria-hidden="true" />
              <div className="min-w-0" aria-live="polite">
                <h3 className="font-serif text-xl font-semibold leading-tight tracking-[-0.02em] text-ink">
                  {presentation.title}
                </h3>
                <p className="mt-1.5 text-sm leading-5 text-ink-secondary">
                  {presentation.summary}
                </p>
              </div>
            </div>

            <div className="mt-4 border-t border-border-subtle pt-3">
              {best && (
                <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
                  <span className="text-sm text-ink-muted">{best.label}</span>
                  <span className="text-2xl font-semibold tabular-nums text-info">{best.value}</span>
                  <span className="text-sm text-ink-muted">{best.detail}</span>
                </div>
              )}
              <div className="mt-3 flex flex-wrap items-center gap-x-5 gap-y-2 text-sm">
                {kept && (
                  <span className="flex items-center gap-1.5 text-ink-secondary">
                    <Check className="size-4 text-info" />
                    <strong className="font-semibold tabular-nums text-info">{kept.value}</strong> kept
                  </span>
                )}
                {undone && (
                  <span className="flex items-center gap-1.5 text-ink-secondary">
                    <RotateCcw className="size-4 text-warning" />
                    <strong className="font-semibold tabular-nums text-warning">{undone.value}</strong> undone
                  </span>
                )}
              </div>
            </div>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-x-2 gap-y-2 border-t border-border px-4 py-2.5 text-xs text-ink-muted">
          <ShieldCheck className={cn("size-4", guardrailFailure ? "text-warning" : "text-info")} />
          <span>{guardrailFailure ? "Guardrail needs attention" : "Guardrails passing"}</span>
          {production && (
            <>
              <span aria-hidden="true">·</span>
              <span>{production.title}</span>
            </>
          )}
          <div className="ml-auto flex items-center gap-1">
            {canPause && (
              <button
                type="button"
                onClick={onPause}
                className="flex min-h-8 items-center gap-1.5 rounded-lg px-2 text-xs font-semibold text-accent transition-colors hover:bg-accent-soft focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-focus"
              >
                <Pause className="size-3.5" /> Pause
              </button>
            )}
            <button
              type="button"
              onClick={() => setExpanded((value) => !value)}
              aria-expanded={expanded}
              aria-controls={detailsId}
              aria-label={expanded ? "Hide RSI loop details" : "Show RSI loop details"}
              className="grid size-8 place-items-center rounded-lg text-ink-muted transition-colors hover:bg-bg-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-focus"
            >
              <ChevronDown className={cn("size-4 transition-transform", expanded && "rotate-180")} />
            </button>
          </div>
        </div>

        {expanded && (
          <div id={detailsId} className="border-t border-border-subtle px-4 py-3">
            <ol className="grid gap-2 sm:grid-cols-5">
              {stages.map((stage) => (
                <li key={stage.id} className="min-w-0 text-xs">
                  <div className={cn(
                    "font-semibold",
                    stage.status === "active" && "text-accent",
                    stage.status === "complete" && "text-info",
                    stage.status === "blocked" && "text-warning",
                    stage.status === "queued" && "text-ink-faint",
                  )}>
                    {stage.label}
                  </div>
                  <p className="mt-1 line-clamp-3 leading-4 text-ink-muted">{stage.detail}</p>
                </li>
              ))}
            </ol>
            <p className="mt-3 border-t border-border-subtle pt-3 text-xs leading-4 text-ink-faint">
              {presentation.limitation}
            </p>
          </div>
        )}
      </div>
    </section>
  );
}
