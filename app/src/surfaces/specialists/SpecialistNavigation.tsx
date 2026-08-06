import {
  FlaskConical,
  Loader2,
  MessageSquare,
  Network,
  ShieldCheck,
  Waypoints,
} from "lucide-react";
import { useSessionStore } from "../../store/sessionStore";
import { useSpecialistStore } from "../../store/specialistStore";
import {
  SPECIALISTS,
  SPECIALIST_KINDS,
  type SpecialistKind,
} from "../../lib/specialists";
import { specialistConversationsForNavigation } from "../../lib/specialistNavigation";
import { cn } from "../../lib/cn";

const ICONS: Readonly<Record<string, typeof Network>> = {
  scout: Network,
  security: ShieldCheck,
  scientist: FlaskConical,
  rsi: Waypoints,
};

const ITEMS = SPECIALIST_KINDS.map((kind) => ({
  kind,
  icon: ICONS[kind] ?? Network,
}));

export function SpecialistNavigation({ rail = false }: { rail?: boolean }) {
  const active = useSpecialistStore((state) => state.active);
  const open = useSpecialistStore((state) => state.open);
  const endSession = useSessionStore((state) => state.endSession);
  const conversations = useSessionStore((state) => state.conversations);
  const sessionId = useSessionStore((state) => state.session?.id ?? null);
  const openingId = useSessionStore((state) => state.opening?.id ?? null);
  const unavailableId = useSessionStore((state) => state.unavailableConversation?.id ?? null);
  const navigatedConversationId = openingId ?? unavailableId ?? sessionId;
  const runningIds = useSessionStore((state) => state.runningIds);
  const openConversation = useSessionStore((state) => state.openConversation);

  const choose = (kind: SpecialistKind) => {
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
    <section data-qa="specialist-navigation" aria-label="Specialist lenses" className="px-2 pb-3 pt-2">
      <div className="px-2 pb-1.5 text-[0.68rem] font-semibold uppercase tracking-[0.12em] text-ink-faint">
        Specialist lenses
      </div>
      <div className="space-y-0.5">
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
              <button
                type="button"
                data-qa={`specialist-nav-${kind}`}
                onClick={() => choose(kind)}
                aria-current={selected ? "page" : undefined}
                className={cn(
                  "relative flex min-h-9 w-full items-center gap-2.5 overflow-hidden border-l-2 px-2 text-sm font-medium transition",
                  selected
                    ? "border-accent bg-accent-subtle text-accent"
                    : "border-transparent text-ink-secondary hover:bg-bg-hover hover:text-ink",
                )}
              >
                <Icon className="relative size-4 shrink-0" />
                <span className="relative">{SPECIALISTS[kind].label}</span>
                <span className="relative ml-auto text-[0.65rem] font-normal text-ink-faint">
                  Pro
                </span>
              </button>
              {selected && specialistConversations.length > 0 && (
                <div className="ml-3 border-l border-border-subtle py-1 pl-2">
                  {specialistConversations.map((conversation) => (
                    <button
                      key={conversation.id}
                      data-qa={`specialist-conversation-${kind}-${conversation.id}`}
                      type="button"
                      onClick={() => void openConversation(conversation.id)}
                      className={cn(
                        "flex min-h-7 w-full items-center gap-1.5 rounded-md px-2 text-left text-xs transition",
                        navigatedConversationId === conversation.id
                          ? "bg-bg-hover text-ink-secondary"
                          : "text-ink-muted hover:bg-bg-hover hover:text-ink",
                      )}
                    >
                      {openingId === conversation.id ? (
                        <Loader2 className="size-3 shrink-0 animate-[spin_1s_linear_infinite]" />
                      ) : (
                        <MessageSquare className={cn(
                          "size-3 shrink-0",
                          runningIds.includes(conversation.id) && "text-accent",
                        )} />
                      )}
                      <span className="truncate">{conversation.title}</span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </section>
  );
}
