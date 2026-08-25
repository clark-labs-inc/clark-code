import { useEffect, useState } from "react";
import { ChevronRight, Loader2, MessageSquare, Network, Trash2 } from "lucide-react";
import { useSessionStore } from "../../store/sessionStore";
import { useSpecialistStore } from "../../store/specialistStore";
import {
  SPECIALISTS,
  SPECIALIST_KINDS,
  type SpecialistKind,
} from "../../lib/specialists";
import type { ConversationMeta } from "../../lib/history";
import { specialistConversationsForNavigation } from "../../lib/specialistNavigation";
import { currentActivity } from "../../lib/activity";
import { cn } from "../../lib/cn";
import { productModule } from "../../product/productModule";

const product = productModule();

const ITEMS = SPECIALIST_KINDS.map((kind) => ({
  kind,
  icon: product.specialistIcons?.[kind] ?? Network,
}));

export function SpecialistConversationRow({
  conversation,
  selected,
  opening,
  running,
  deleting,
  confirmingDelete,
  onOpen,
  onRequestDelete,
  onConfirmDelete,
  onCancelDelete,
}: {
  conversation: ConversationMeta;
  selected: boolean;
  opening: boolean;
  running: boolean;
  deleting: boolean;
  confirmingDelete: boolean;
  onOpen: () => void;
  onRequestDelete: () => void;
  onConfirmDelete: () => void;
  onCancelDelete: () => void;
}) {
  const working = opening || running || deleting;
  return (
    <div
      data-qa={`specialist-conversation-${conversation.specialist?.kind}-${conversation.id}`}
      aria-busy={working || undefined}
      className={cn(
        "group relative flex h-8 w-full items-center gap-1 rounded-md px-1 text-sm leading-none transition",
        selected
          ? "bg-accent-subtle font-medium text-ink"
          : "text-ink-muted hover:bg-bg-hover hover:text-ink",
      )}
    >
      <button
        type="button"
        onClick={onOpen}
        disabled={deleting}
        aria-current={selected ? "page" : undefined}
        aria-label={`${deleting ? "Deleting " : ""}${conversation.title}${selected ? ", selected" : ""}${running ? ", Clark is working" : ""}`}
        className="flex min-w-0 flex-1 items-center gap-2 px-1 text-left disabled:cursor-wait"
      >
        {working ? (
          <Loader2 className="size-3.5 shrink-0 animate-[spin_1s_linear_infinite] text-accent" aria-hidden="true" />
        ) : (
          <MessageSquare
            className={cn("size-3.5 shrink-0", selected && "text-accent")}
            aria-hidden="true"
          />
        )}
        <span className="min-w-0 flex-1 truncate">{conversation.title}</span>
        {running && (
          <span className="breathe size-1.5 shrink-0 rounded-full bg-accent" aria-hidden="true" />
        )}
      </button>
      {confirmingDelete ? (
        <span className="flex shrink-0 items-center gap-0.5">
          <button
            type="button"
            data-qa={`specialist-delete-confirm-${conversation.id}`}
            onClick={onConfirmDelete}
            disabled={deleting}
            aria-label={`Permanently delete ${conversation.title}`}
            className="rounded px-1 py-1 font-medium text-danger transition hover:bg-danger/10 disabled:cursor-wait"
          >
            Delete
          </button>
          <button
            type="button"
            onClick={onCancelDelete}
            disabled={deleting}
            aria-label="Cancel delete"
            className="rounded px-1 py-1 text-ink-faint transition hover:bg-bg-sunken hover:text-ink disabled:cursor-wait"
          >
            Cancel
          </button>
        </span>
      ) : (
        <button
          type="button"
          data-qa={`specialist-delete-${conversation.id}`}
          onClick={onRequestDelete}
          disabled={deleting}
          title="Delete conversation"
          aria-label={`Delete ${conversation.title}`}
          className="grid size-6 shrink-0 place-items-center rounded text-ink-faint opacity-0 transition hover:bg-danger/10 hover:text-danger group-hover:opacity-100 group-focus-within:opacity-100 disabled:cursor-wait"
        >
          <Trash2 className="size-3.5" />
        </button>
      )}
    </div>
  );
}

export function SpecialistNavigation({ rail = false }: { rail?: boolean }) {
  const active = useSpecialistStore((state) => state.active);
  const expanded = useSpecialistStore((state) => state.expanded);
  const open = useSpecialistStore((state) => state.open);
  const endSession = useSessionStore((state) => state.endSession);
  const conversations = useSessionStore((state) => state.conversations);
  const sessionId = useSessionStore((state) => state.session?.id ?? null);
  const openingId = useSessionStore((state) => state.opening?.id ?? null);
  const unavailableId = useSessionStore((state) => state.unavailableConversation?.id ?? null);
  const navigatedConversationId = openingId ?? unavailableId ?? sessionId;
  const runningIds = useSessionStore((state) => state.runningIds);
  const mutatingIds = useSessionStore((state) => state.mutatingConversationIds);
  const conversationMutation = useSessionStore((state) => state.conversationMutation);
  const activeConversationBusy = useSessionStore((state) => currentActivity(state.snapshot).busy);
  const openConversation = useSessionStore((state) => state.openConversation);
  const deleteConversation = useSessionStore((state) => state.deleteConversation);
  const [confirmingDeleteId, setConfirmingDeleteId] = useState<string | null>(null);
  const [navigationOpen, setNavigationOpen] = useState(() => active !== null);

  // Ordinary conversations should get the sidebar's full height. Keep the
  // last expanded specialist branch in the store so reopening this section
  // still lands on the same saved specialist history.
  useEffect(() => {
    setNavigationOpen(active !== null);
  }, [active]);

  const choose = (kind: SpecialistKind) => {
    setConfirmingDeleteId(null);
    setNavigationOpen(true);
    if (active !== kind) endSession();
    open(kind);
  };

  if (rail) {
    return (
      <div className="mt-2 flex flex-col gap-1 border-t border-border-subtle pt-2">
        {ITEMS.map(({ kind, icon: Icon }) => (
          <button
            key={kind}
            data-qa={`specialist-nav-${kind}`}
            type="button"
            onClick={() => choose(kind)}
            aria-label={`Open ${SPECIALISTS[kind].label}`}
            title={`${SPECIALISTS[kind].label} — ${SPECIALISTS[kind].value}`}
            className={cn(
              "grid size-8 place-items-center rounded-lg transition",
              active === kind
                ? "bg-accent-soft text-accent"
                : "text-ink-muted hover:bg-bg-hover hover:text-ink",
            )}
          >
            <Icon className="size-4" />
          </button>
        ))}
      </div>
    );
  }

  return (
    <section data-qa="specialist-navigation" aria-label="Specialist lenses" className="px-2 pb-2 pt-1">
      <button
        type="button"
        onClick={() => setNavigationOpen((open) => !open)}
        aria-controls="specialist-navigation-list"
        aria-expanded={navigationOpen}
        className="flex h-7 w-full items-center gap-1.5 rounded-md px-2 text-left text-sm font-semibold uppercase leading-none tracking-[0.12em] text-ink-faint transition hover:bg-bg-hover hover:text-ink"
      >
        <ChevronRight
          className={cn("size-3 shrink-0 transition-transform", navigationOpen && "rotate-90")}
          aria-hidden="true"
        />
        Specialist lenses
        {!navigationOpen && active && (
          <span className="ml-auto truncate text-sm font-normal normal-case tracking-normal text-ink-muted">
            {SPECIALISTS[active].label}
          </span>
        )}
      </button>
      {navigationOpen && (
        <div id="specialist-navigation-list" className="space-y-0.5">
          {ITEMS.map(({ kind, icon: Icon }) => {
            const selected = active === kind;
            // Access gates paid specialist actions, not the user's navigation
            // history. Keep saved chats visible during entitlement checks,
            // downgrades, and outages so a selected recovery target never vanishes.
            const specialistConversations = specialistConversationsForNavigation(
              conversations,
              kind,
            );
            return (
              <div key={kind}>
                <div className="flex h-9 items-center">
                  <button
                    type="button"
                    data-qa={`specialist-nav-${kind}`}
                    onClick={() => choose(kind)}
                    aria-current={selected ? "page" : undefined}
                    className={cn(
                      "flex h-full min-w-0 flex-1 items-center gap-2.5 rounded-md px-2 text-base font-medium leading-none transition",
                      selected
                        ? "text-ink"
                        : "text-ink-secondary hover:bg-bg-hover hover:text-ink",
                    )}
                  >
                    <Icon className={cn("size-[18px] shrink-0", selected && "text-accent")} />
                    <span className="truncate">{SPECIALISTS[kind].label}</span>
                  </button>
                </div>
                {expanded === kind && specialistConversations.length > 0 && (
                  <div className="relative ml-[17px] border-l border-accent/40 py-0.5 pl-[14px]">
                    {specialistConversations.map((conversation) => (
                      <div
                        key={conversation.id}
                        className="relative before:absolute before:-left-[14px] before:top-1/2 before:w-[14px] before:border-t before:border-accent/40"
                      >
                        <SpecialistConversationRow
                          conversation={conversation}
                          selected={navigatedConversationId === conversation.id}
                          opening={openingId === conversation.id}
                          running={
                            runningIds.includes(conversation.id)
                            || (sessionId === conversation.id && activeConversationBusy)
                          }
                          deleting={
                            mutatingIds.has(conversation.id)
                            && conversationMutation?.kind === "delete"
                          }
                          confirmingDelete={confirmingDeleteId === conversation.id}
                          onOpen={() => {
                            setConfirmingDeleteId(null);
                            void openConversation(conversation.id);
                          }}
                          onRequestDelete={() => setConfirmingDeleteId(conversation.id)}
                          onConfirmDelete={() => {
                            setConfirmingDeleteId(null);
                            void deleteConversation(conversation.id);
                          }}
                          onCancelDelete={() => setConfirmingDeleteId(null)}
                        />
                      </div>
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}
