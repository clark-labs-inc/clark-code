import { useId, useState } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import {
  Activity,
  ArrowRight,
  ChartColumn,
  FlaskConical,
  Gauge,
  GitBranch,
  GitCompare,
  Map,
  Radar,
  Repeat2,
  Route,
  ShieldCheck,
  TriangleAlert,
  type LucideIcon,
} from "lucide-react";

import {
  SPECIALISTS,
  type SpecialistKind,
  type SpecialistTab,
  type SpecialistWorkflow,
} from "../../lib/specialists";
import {
  DUR,
  RISE_SMALL,
  SLIDE_LEFT,
  SLIDE_RIGHT,
  accessibleMotion,
  staggeredTransition,
} from "../../lib/motion";
import { cn } from "../../lib/cn";
import { SpecialistConversationShowcase } from "./SpecialistConversationShowcase";

export interface SpecialistStarter {
  title: string;
  detail: string;
  prompt: string;
  tab: SpecialistTab;
  workflow: SpecialistWorkflow;
  icon: LucideIcon;
}

const STARTERS: Record<SpecialistKind, readonly SpecialistStarter[]> = {
  security: [
    {
      title: "Deep scan this repository",
      detail: "Trace exploitable paths across multiple independent passes.",
      prompt: "Deep scan the current repository and prioritize exploitable paths.",
      tab: "scans",
      workflow: "security:security-deep",
      icon: Radar,
    },
    {
      title: "Review current changes",
      detail: "Check the working diff for security regressions.",
      prompt: "Review the current diff for security regressions and show the supporting evidence.",
      tab: "scans",
      workflow: "security:security-diff",
      icon: GitCompare,
    },
    {
      title: "Assess repository posture",
      detail: "Establish coverage and prioritize validated findings.",
      prompt: "Assess the security posture of this repository and prioritize validated findings.",
      tab: "posture",
      workflow: "security:security-scan",
      icon: ShieldCheck,
    },
  ],
  scout: [
    {
      title: "Map this organization",
      detail: "Census authorized sources, repositories, systems, and dependencies.",
      prompt: "Map the selected organization and workspace. Begin with an adapter and authenticated-context census, reconcile remote repositories with local checkouts, and show evidence and coverage gaps for the resulting system graph.",
      tab: "map",
      workflow: "scout:scout",
      icon: Map,
    },
    {
      title: "Assess a proposed change",
      detail: "Trace impact through the selected enterprise graph before implementation.",
      prompt: "Assess the downstream impact of the change I describe against the selected Scout workspace and identify uncertain, stale, or inaccessible dependencies.",
      tab: "changes",
      workflow: "scout:scout",
      icon: GitBranch,
    },
    {
      title: "Simulate an outage",
      detail: "Explore blast radius, fallbacks, and recovery evidence.",
      prompt: "What breaks if the identity service is unavailable?",
      tab: "simulations",
      workflow: "scout:scout",
      icon: Activity,
    },
  ],
  scientist: [
    {
      title: "Launch a discovery program",
      detail: "Turn a broad objective into preregistered, falsifiable studies.",
      prompt: "Create a research program for this objective, preregister the first discriminating study, and identify the evidence needed before any experiment runs.",
      tab: "programs",
      workflow: "scientist:discover",
      icon: FlaskConical,
    },
    {
      title: "Challenge an existing result",
      detail: "Design independent replications and adversarial falsification tests.",
      prompt: "Audit the strongest current claim, design an independent replication, and prioritize the most informative falsification attempt.",
      tab: "experiments",
      workflow: "scientist:replicate",
      icon: Repeat2,
    },
    {
      title: "Synthesize the evidence",
      detail: "Separate observations, claims, limitations, and unresolved uncertainty.",
      prompt: "Synthesize the available evidence into supported, refuted, and unresolved claims with explicit limitations.",
      tab: "evidence",
      workflow: "scientist:discover",
      icon: ChartColumn,
    },
  ],
  rsi: [
    {
      title: "Create a great evaluation",
      detail: "Research the target and generate high-information evaluations.",
      prompt: "Research this target, map its important behaviors with Scout, create a bounded evaluation world, and run the highest-information safe evaluation.",
      tab: "evaluations",
      workflow: "rsi:create-evals",
      icon: TriangleAlert,
    },
    {
      title: "Build a regression world",
      detail: "Turn incidents, invariants, and memory into deterministic coverage.",
      prompt: "Build a reproducible regression evaluation world from target invariants, incidents, Scout context, and known failure modes.",
      tab: "frontier",
      workflow: "rsi:regression",
      icon: Gauge,
    },
    {
      title: "Build an evaluation world",
      detail: "Model actors, perturbations, telemetry, safety, and oracles.",
      prompt: "Build an evaluation world with explicit actors, perturbations, telemetry, safety constraints, and measurable independent oracles.",
      tab: "worlds",
      workflow: "rsi:build-world",
      icon: Route,
    },
  ],
};

export function specialistStarters(kind: SpecialistKind): readonly SpecialistStarter[] {
  return STARTERS[kind] ?? [];
}

export function SpecialistWelcome({
  kind,
  onStart,
}: {
  kind: SpecialistKind;
  onStart: (starter: SpecialistStarter) => void;
}) {
  const reduceMotion = useReducedMotion();
  const introductionId = useId();
  const [mode, setMode] = useState<"start" | "example">("start");
  const definition = SPECIALISTS[kind];
  const workspaceCopy = {
    scout: "Choose a Clark organization and Scout workspace, then explicitly start a run. Scout maps authorized source, delivery, runtime, data, identity, ownership, and observability evidence without treating the currently open folder as the system boundary.",
    security: "Choose a repository-level investigation. Security keeps coverage, validated findings, evidence, and remediation organized in the canvas.",
    scientist: "Describe the discovery you want to pursue. Scientist separates hypotheses, experiments, observations, claims, replications, and decisions.",
    rsi: "Describe a project, product, environment, or model. RSI uses research context to build evaluation worlds, search high-information tests, and preserve reproducible counterexamples.",
  }[kind] ?? definition.value;

  return (
    <div
      data-qa={`specialist-welcome-${kind}`}
      className="specialist-welcome mx-auto w-full max-w-xl"
    >
      <h2
        className={cn(
          "font-serif text-2xl font-semibold leading-[1.05] tracking-[-0.025em] text-ink",
          mode === "example" && "hidden xl:block",
        )}
      >
        {definition.headline}
      </h2>

      <p className={cn(
        "specialist-welcome-copy mt-3 max-w-lg text-sm leading-6 text-ink-secondary",
        mode === "example" && "hidden xl:block",
      )}>
        {workspaceCopy}
      </p>

      <div
        role="tablist"
        aria-label={`${definition.label} conversation introduction`}
        className="specialist-welcome-tabs mt-6 flex w-fit items-center gap-5"
      >
        {([
          ["start", "Start a conversation"],
          ["example", "Example analysis"],
        ] as const).map(([id, label]) => (
          <button
            key={id}
            data-qa={`specialist-intro-${kind}-${id}`}
            id={`${introductionId}-${id}-tab`}
            type="button"
            role="tab"
            aria-selected={mode === id}
            aria-controls={`${introductionId}-${id}-panel`}
            onClick={() => setMode(id)}
            className={cn(
              "specialist-tab min-h-8 border-b-2 px-0 text-xs font-medium transition-colors",
              mode === id
                ? "border-accent text-ink"
                : "border-transparent text-ink-muted hover:text-ink",
            )}
          >
            {label}
          </button>
        ))}
      </div>

      <AnimatePresence mode="wait" initial={false}>
        {mode === "start" ? (
          <m.div
            key="start"
            id={`${introductionId}-start-panel`}
            role="tabpanel"
            aria-labelledby={`${introductionId}-start-tab`}
            {...accessibleMotion(SLIDE_LEFT, reduceMotion)}
            transition={staggeredTransition(reduceMotion, 0, 0.04, { duration: DUR.fast })}
            className="specialist-welcome-panel mt-5"
          >
            <div className="mb-2 text-xs font-semibold text-ink-muted">Choose a starting point</div>
            <div className="specialist-welcome-starters flex flex-col gap-1">
              {specialistStarters(kind).map((starter, index) => {
                const StarterIcon = starter.icon;
                return (
                  <m.button
                    key={starter.title}
                    data-qa={`specialist-starter-${kind}-${index}`}
                    aria-label={`Start ${definition.label}: ${starter.title}`}
                    title={starter.detail}
                    type="button"
                    {...accessibleMotion(RISE_SMALL, reduceMotion)}
                    transition={staggeredTransition(reduceMotion, index, 0.035)}
                    onClick={() => onStart(starter)}
                    className="specialist-welcome-starter group flex w-full items-center gap-3 rounded-xl px-2 py-2.5 text-left transition-colors hover:bg-bg-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent/40"
                  >
                    <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-accent-soft text-accent transition-transform group-hover:scale-[1.03]">
                      <StarterIcon className="size-4" />
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block text-sm font-medium text-ink">{starter.title}</span>
                      <span className="specialist-welcome-starter-detail mt-0.5 block text-xs leading-5 text-ink-muted">
                        {starter.detail}
                      </span>
                    </span>
                    <ArrowRight className="mr-1 size-4 shrink-0 text-ink-faint transition-transform group-hover:translate-x-0.5 group-hover:text-accent" />
                  </m.button>
                );
              })}
            </div>
            <p className="specialist-welcome-footer mt-3 text-xs leading-5 text-ink-faint">
              Or describe your own investigation below. Nothing runs until you send it.
            </p>
          </m.div>
        ) : (
          <m.div
            key="example"
            id={`${introductionId}-example-panel`}
            role="tabpanel"
            aria-labelledby={`${introductionId}-example-tab`}
            {...accessibleMotion(SLIDE_RIGHT, reduceMotion)}
            transition={staggeredTransition(reduceMotion, 0, 0.04, { duration: DUR.fast })}
            className="mt-5"
          >
            <SpecialistConversationShowcase
              kind={kind}
              onUsePrompt={(prompt) => {
                const starter = specialistStarters(kind)[0];
                if (starter) onStart(prompt ? { ...starter, prompt } : starter);
              }}
            />
          </m.div>
        )}
      </AnimatePresence>
    </div>
  );
}
