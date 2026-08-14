import { useEffect, useState } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { Archive, Check, Loader2, Trash2 } from "lucide-react";
import { DIALOG, OVERLAY, accessibleMotion } from "../lib/motion";
import { useSessionStore } from "../store/sessionStore";

type ExitKind = "archive" | "delete";

interface VisibleTransition {
  mutationId: number;
  kind: ExitKind;
  phase: "working" | "done";
}

const DONE_HOLD_MS = 520;

/**
 * Keeps an active conversation visually accounted for while archive/delete
 * moves it out of the workspace. The store intentionally clears the session
 * as soon as the durable mutation succeeds; this brief layer masks that hard
 * content swap and explains the same transition shown by the sidebar row.
 */
export function ConversationMutationTransition() {
  const visibleConversationId = useSessionStore(
    (state) => state.opening?.id ?? state.unavailableConversation?.id ?? state.session?.id ?? null,
  );
  const mutation = useSessionStore((state) => state.conversationMutation);
  const visibleConversationIsMutating = useSessionStore((state) =>
    visibleConversationId ? state.mutatingConversationIds.has(visibleConversationId) : false,
  );
  const reduceMotion = useReducedMotion();
  const activeKind =
    visibleConversationIsMutating
    && (mutation?.kind === "archive" || mutation?.kind === "delete")
      ? mutation.kind
      : null;
  const activeMutationId = activeKind ? mutation?.id ?? null : null;
  const [transition, setTransition] = useState<VisibleTransition | null>(null);

  useEffect(() => {
    if (!activeKind || activeMutationId === null) return;
    setTransition((current) =>
      current?.mutationId === activeMutationId
      && current.kind === activeKind
      && current.phase === "working"
        ? current
        : { mutationId: activeMutationId, kind: activeKind, phase: "working" },
    );
  }, [activeKind, activeMutationId]);

  useEffect(() => {
    if (!transition || activeMutationId !== null) return;
    if (transition.phase === "working") {
      setTransition({ ...transition, phase: "done" });
      return;
    }
    const timeout = window.setTimeout(() => setTransition(null), DONE_HOLD_MS);
    return () => window.clearTimeout(timeout);
  }, [activeMutationId, transition]);

  const visible = activeKind && activeMutationId !== null
    ? { mutationId: activeMutationId, kind: activeKind, phase: "working" as const }
    : transition;
  const deleting = visible?.kind === "delete";
  const working = visible?.phase === "working";
  const label = visible
    ? working
      ? `${deleting ? "Deleting" : "Archiving"} conversation…`
      : `Conversation ${deleting ? "deleted" : "archived"}`
    : "";

  return (
    <AnimatePresence initial={false}>
      {visible && (
        <m.div
          key={visible.mutationId}
          role="status"
          aria-live="polite"
          aria-atomic="true"
          aria-label={label}
          data-conversation-mutation-transition={visible.kind}
          {...accessibleMotion(OVERLAY, reduceMotion)}
          className="absolute inset-0 z-30 grid cursor-wait place-items-center bg-bg/45 backdrop-blur-[2px]"
        >
          <m.div
            {...accessibleMotion(DIALOG, reduceMotion)}
            className="flex min-w-56 items-center gap-3 rounded-2xl border border-border-subtle bg-bg-elevated/95 px-4 py-3 text-sm font-medium text-ink shadow-elevated"
          >
            <span className="relative grid size-8 shrink-0 place-items-center rounded-full bg-bg-tertiary text-ink-muted">
              {working ? (
                <Loader2 className="size-4 animate-[spin_1s_linear_infinite]" aria-hidden="true" />
              ) : (
                <Check className="size-4 text-success" aria-hidden="true" />
              )}
              {working && (deleting
                ? <Trash2 className="absolute size-3" aria-hidden="true" />
                : <Archive className="absolute size-3" aria-hidden="true" />)}
            </span>
            <span>{label}</span>
          </m.div>
        </m.div>
      )}
    </AnimatePresence>
  );
}
