import { lazy, Suspense } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { useSessionStore } from "../../store/sessionStore";
import { useSpecialistStore } from "../../store/specialistStore";
import { SPECIALISTS, type SpecialistKind } from "../../lib/specialists";
import { RISE, accessibleMotion } from "../../lib/motion";
import { PanelErrorBoundary } from "../../components/PanelErrorBoundary";
import { GoalStatusRail } from "../GoalStatusRail";
import { Composer } from "../Composer";
import { SpecialistWelcome, type SpecialistStarter } from "./SpecialistWelcome";

const Conversation = lazy(() =>
  import("../Conversation").then((module) => ({ default: module.Conversation })),
);

export function ContextualConversation({ kind }: { kind: SpecialistKind }) {
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
