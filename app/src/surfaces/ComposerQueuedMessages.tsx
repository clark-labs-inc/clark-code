import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { CornerDownRight, Pencil, X } from "lucide-react";
import { useSessionStore, type QueuedMessage } from "../store/sessionStore";
import { EXPAND, EXPAND_REDUCED } from "../lib/motion";

/** Messages typed while a run is active. They send automatically, in order,
 * when the run finishes — no interruption. Each can be edited or dropped. */
export function ComposerQueuedMessages({ onEdit }: { onEdit: (q: QueuedMessage) => void }) {
  const reduce = useReducedMotion();
  const queued = useSessionStore((s) => s.queued);
  const session = useSessionStore((s) => s.session);
  const busy = useSessionStore((s) =>
    Object.values(s.snapshot.runs).some((r) => r.status === "running" || r.status === "queued"),
  );
  const steerQueued = useSessionStore((s) => s.steerQueued);
  const removeQueued = useSessionStore((s) => s.removeQueued);
  if (queued.length === 0) return null;
  return (
    <div className="conversation-column-width mx-auto mb-2 w-full">
      <div className="mb-1 px-1 text-xs font-medium uppercase tracking-wide text-ink-faint">
        Queued · sends when the agent finishes
      </div>
      <div className="space-y-1">
        <AnimatePresence initial={false}>
          {queued.map((q) => (
            <m.div
              key={q.id}
              layout={!reduce}
              {...(reduce ? EXPAND_REDUCED : EXPAND)}
              className="group flex items-center gap-2 overflow-hidden rounded-xl bg-accent-subtle py-2 pl-3 pr-2"
            >
              <CornerDownRight className="size-3.5 shrink-0 text-ink-faint" />
              <span className="min-w-0 flex-1 truncate text-xs text-ink-secondary">
                {q.text || (q.skills.length > 0 ? "(skills selected)" : "(attachments only)")}
              </span>
              <span className="flex shrink-0 items-center gap-0.5">
                {session?.provider === "local"
                  && busy
                  && q.uploads.length === 0
                  && q.skills.length === 0
                  && (
                  <button
                    onClick={() => void steerQueued(q.id)}
                    aria-label="Steer active run with queued message"
                    title="Send now and steer the active run"
                    className="flex h-6 items-center gap-1 rounded-md px-1.5 text-xs text-ink-muted transition hover:bg-bg-hover hover:text-ink"
                  >
                    <CornerDownRight className="size-3" />
                    Steer
                  </button>
                )}
                <button
                  onClick={() => onEdit(q)}
                  aria-label="Edit queued message"
                  className="grid size-6 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink"
                >
                  <Pencil className="size-3.5" />
                </button>
                <button
                  onClick={() => removeQueued(q.id)}
                  aria-label="Remove queued message"
                  className="grid size-6 place-items-center rounded-md text-ink-muted transition hover:bg-danger/15 hover:text-danger"
                >
                  <X className="size-3.5" />
                </button>
              </span>
            </m.div>
          ))}
        </AnimatePresence>
      </div>
    </div>
  );
}
