/** One conversation row in the sidebar list.
 *
 *  Extracted from `Sidebar.tsx` and memoized. Every row is wrapped in a
 *  layout-animated Motion node, and Motion re-measures a layout node's subtree
 *  whenever it re-renders — so an unmemoized row made any sidebar state change
 *  cost a `getBoundingClientRect` per row. With 120 conversations that measured
 *  25 ms to repaint a change that affected exactly one row.
 *
 *  The comparator only works while the callback props keep a stable identity;
 *  `Sidebar` holds them through `useCallback` with a latest-value ref for that
 *  reason. Adding a prop here means adding it to the comparator below. */
import { memo, useState } from "react";
import * as m from "motion/react-m";
import { useReducedMotion } from "motion/react";
import { Archive, Loader2, MessageSquare } from "lucide-react";
import type React from "react";

import { useSessionStore } from "../../store/sessionStore";
import { cn } from "../../lib/cn";
import type { ConversationMeta } from "../../lib/history";
import type {
  ConversationSelectionIntent,
  SidebarConversationMutationKind,
} from "../../lib/sidebarConversationInteractions";
import { DUR, RISE_SMALL, accessibleMotion, staggeredTransition } from "../../lib/motion";

function ConversationRowImpl({
  c,
  active,
  streaming,
  unseen,
  opening,
  selected,
  mutation,
  onSelect,
  onRangeStep,
  onArchive,
  onContextMenu,
}: {
  c: ConversationMeta;
  active: boolean;
  /** A run is currently streaming in this conversation. */
  streaming: boolean;
  /** Finished in the background and not opened yet — the blue "done" dot. */
  unseen: boolean;
  /** This conversation is currently being (re)opened. */
  opening: boolean;
  /** In the sidebar's Shift-click selection. */
  selected: boolean;
  mutation: SidebarConversationMutationKind | null;
  onSelect: (id: string, intent: ConversationSelectionIntent, additive?: boolean) => void;
  onRangeStep: (id: string, direction: -1 | 1) => void;
  onArchive: (id: string) => void;
  onContextMenu: (e: React.MouseEvent, id: string) => void;
}) {
  const reduceMotion = useReducedMotion();
  const rename = useSessionStore((s) => s.renameConversation);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(c.title);
  const mutating = mutation !== null;

  const commit = () => {
    setEditing(false);
    rename(c.id, draft);
  };

  return (
    // The layout-animated wrapper belongs inside the memo boundary: Motion
    // re-measures a layout node whenever it re-renders, so leaving this at the
    // call site meant every sidebar state change still cost a projection pass
    // per row even though the row's own content was memoized.
    <m.div
      data-sidebar-conversation-id={c.id}
      layout={reduceMotion ? false : "position"}
      {...accessibleMotion(RISE_SMALL, reduceMotion)}
      transition={staggeredTransition(reduceMotion, 0, 0.04, { duration: DUR.fast })}
    >
      <div
        onContextMenu={(e) => {
          if (!mutating) onContextMenu(e, c.id);
        }}
        aria-busy={mutating || undefined}
        className={cn(
          "group relative flex min-h-7 items-center gap-1 rounded-lg px-2 py-0.5 text-base transition duration-fast ease-agent",
          mutating && "opacity-60",
          selected
            ? "bg-bg-tertiary text-ink"
            : active || opening
              ? "bg-bg-tertiary text-ink"
              : "text-ink-secondary hover:bg-bg-hover hover:text-ink",
        )}
      >
        {editing ? (
          <div className="flex min-w-0 flex-1 items-center gap-2">
            <MessageSquare className="size-3.5 shrink-0 text-ink-faint" />
            <input
              autoFocus
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={commit}
              onKeyDown={(e) => {
                if (e.key === "Enter") commit();
                if (e.key === "Escape") {
                  setDraft(c.title);
                  setEditing(false);
                }
              }}
              aria-label="Rename conversation"
              autoCorrect="off"
              autoCapitalize="off"
              spellCheck={false}
              className="composer-input min-w-0 flex-1 rounded-md bg-bg-sunken px-1.5 py-0.5 text-base text-ink outline-none ring-1 ring-border-subtle"
            />
          </div>
        ) : (
          <button
            onClick={(e) => {
              const additive = e.metaKey || e.ctrlKey;
              if (e.shiftKey) {
                onSelect(c.id, "range", additive);
              } else if (additive) {
                onSelect(c.id, "toggle");
              } else {
                onSelect(c.id, "open");
              }
            }}
            onKeyDown={(e) => {
              if (e.shiftKey && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
                e.preventDefault();
                onRangeStep(c.id, e.key === "ArrowDown" ? 1 : -1);
                return;
              }
              if (e.key === " " && (e.shiftKey || e.metaKey || e.ctrlKey)) {
                e.preventDefault();
                onSelect(c.id, e.shiftKey ? "range" : "toggle", e.metaKey || e.ctrlKey);
              }
            }}
            onDoubleClick={() => {
              setDraft(c.title);
              setEditing(true);
            }}
            disabled={mutating}
            data-sidebar-conversation-button={c.id}
            aria-pressed={selected}
            aria-describedby="sidebar-selection-help"
            aria-label={
              mutating
                ? `${mutation === "archive" ? "Archiving" : mutation === "delete" ? "Deleting" : "Restoring"} ${c.title}`
                : `Conversation: ${c.title}${unseen ? ", has finished work you haven't reviewed" : ""}${selected ? ", selected" : ""}`
            }
            className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
            title={`${c.title} — Shift-click or Shift+Arrow selects a range`}
          >
            {mutating || opening ? (
              <Loader2 className="size-3.5 shrink-0 animate-[spin_1s_linear_infinite] text-ink-muted" />
            ) : streaming ? (
              <span className="relative grid size-3.5 shrink-0 place-items-center" aria-hidden="true">
                <span className="absolute size-2 animate-ping rounded-full bg-accent/40" />
                <span className="size-1.5 rounded-full bg-accent" />
              </span>
            ) : unseen ? (
              <span className="grid size-3.5 shrink-0 place-items-center" aria-hidden="true">
                <span className="size-2 rounded-full bg-info" />
              </span>
            ) : selected ? (
              <span className="grid size-3 shrink-0 place-items-center" aria-hidden="true">
                <span className="size-2 rounded-sm bg-accent" />
              </span>
            ) : (
              <span className="size-3 shrink-0" aria-hidden="true" />
            )}
            <span className="min-w-0 flex-1 truncate leading-5">{c.title}</span>
          </button>
        )}
        {!editing && (
          <button
            onClick={() => onArchive(c.id)}
            disabled={mutating}
            title="Archive conversation"
            aria-label="Archive conversation"
            className="shrink-0 rounded-md p-1 text-ink-faint opacity-0 transition hover:bg-bg-sunken hover:text-ink group-hover:opacity-100 group-focus-within:opacity-100 disabled:cursor-wait"
          >
            <Archive className="size-3.5" />
          </button>
        )}
      </div>
    </m.div>
  );
}

export const ConversationRow = memo(
  ConversationRowImpl,
  (a, b) =>
    a.c === b.c
    && a.active === b.active
    && a.streaming === b.streaming
    && a.unseen === b.unseen
    && a.opening === b.opening
    && a.selected === b.selected
    && a.mutation === b.mutation
    && a.onSelect === b.onSelect
    && a.onRangeStep === b.onRangeStep
    && a.onArchive === b.onArchive
    && a.onContextMenu === b.onContextMenu,
);
