import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { ChevronRight } from "lucide-react";
import { ArchivedRow } from "./ArchivedRow";
import { DUR, EXPAND, EXPAND_REDUCED, RISE_SMALL, accessibleMotion, staggeredTransition } from "../../lib/motion";
import type { ConversationMeta } from "../../lib/history";
import type { SidebarConversationMutationKind } from "../../lib/sidebarConversationInteractions";

export function SidebarArchive({ archivedConvos, open, onToggle, mutatingIds, mutationKind, onRestore, onDelete }: {
  archivedConvos: ConversationMeta[];
  open: boolean;
  onToggle: () => void;
  mutatingIds: Set<string>;
  mutationKind: SidebarConversationMutationKind | undefined;
  onRestore: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  const reduceMotion = useReducedMotion();
  return (
      <div className="shrink-0 pt-1">
        <button
          onClick={() => onToggle()}
          disabled={archivedConvos.length === 0}
          aria-controls="archived-conversations"
          aria-expanded={archivedConvos.length > 0 ? open : undefined}
          className="flex min-h-9 w-full items-center gap-2 px-4 py-1 text-base font-medium text-ink-muted transition hover:text-ink disabled:cursor-default disabled:opacity-55"
        >
          <ChevronRight
            className={`size-3 shrink-0 transition-transform ${(open) && archivedConvos.length > 0 ? "rotate-90" : ""}`}
          />
          <span>Archived</span>
          <span className="ml-auto shrink-0 text-sm font-normal tabular-nums text-ink-faint">
            {archivedConvos.length}
          </span>
        </button>
        <AnimatePresence initial={false}>
          {(open) && archivedConvos.length > 0 && (
            <m.div
              id="archived-conversations"
              {...(reduceMotion ? EXPAND_REDUCED : EXPAND)}
              className="overflow-hidden"
            >
              <div className="flex max-h-56 flex-col gap-1 overflow-y-auto px-2 pb-2">
                <AnimatePresence initial={false} mode="popLayout">
                  {archivedConvos.map((c) => {
                    const mutation = mutatingIds.has(c.id)
                      ? mutationKind ?? "restore"
                      : null;
                    return (
                      <m.div
                        key={c.id}
                        layout={reduceMotion ? false : "position"}
                        {...accessibleMotion(RISE_SMALL, reduceMotion)}
                        transition={staggeredTransition(reduceMotion, 0, 0.04, { duration: DUR.fast })}
                      >
                        <ArchivedRow
                          c={c}
                          mutation={mutation}
                          onRestore={onRestore}
                          onDelete={onDelete}
                        />
                      </m.div>
                    );
                  })}
                </AnimatePresence>
              </div>
            </m.div>
          )}
        </AnimatePresence>
      </div>

  );
}
