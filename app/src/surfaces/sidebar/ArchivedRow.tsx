/** The dimmed row inside the collapsed "Archived" tray.
 *
 *  Extracted from `Sidebar.tsx` and memoized for the same reason as
 *  `ConversationRow`: the archived list is also layout-animated, so an
 *  unmemoized row makes Motion re-measure the tray on unrelated state changes. */
import { memo, useState } from "react";
import { ArchiveRestore, Loader2, MessageSquare, Trash2 } from "lucide-react";

import type { ConversationMeta } from "../../lib/history";
import type { SidebarConversationMutationKind } from "../../lib/sidebarConversationInteractions";

/** A dimmed, minimal row inside the collapsed "Archived" section. Clicking the
 *  row restores the conversation (returns it to the active list); the trash
 *  button permanently deletes it (local cache + cloud) behind an inline confirm,
 *  since that can't be undone. */
function ArchivedRowImpl({
  c,
  mutation,
  onRestore,
  onDelete,
}: {
  c: ConversationMeta;
  mutation: SidebarConversationMutationKind | null;
  onRestore: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const mutating = mutation !== null;
  return (
    <div
      aria-busy={mutating || undefined}
      className="group flex min-h-7 w-full items-center gap-1 rounded-lg px-2 py-0.5 text-base text-ink-faint transition hover:bg-bg-hover"
    >
      <button
        onClick={() => onRestore(c.id)}
        disabled={mutating}
        title={`Restore “${c.title}”`}
        aria-label={mutating ? `Restoring ${c.title}` : `Restore ${c.title} and open it`}
        className="flex min-w-0 flex-1 items-center gap-1.5 text-left transition hover:text-ink-secondary"
      >
        {mutating ? (
          <Loader2 className="size-3.5 shrink-0 animate-[spin_1s_linear_infinite] text-ink-muted" />
        ) : (
          <MessageSquare className="size-3.5 shrink-0 text-ink-faint" />
        )}
        <span className="min-w-0 flex-1 truncate leading-5">{c.title}</span>
        <ArchiveRestore className="size-3.5 shrink-0 opacity-0 transition group-hover:opacity-100" />
      </button>
      {confirming ? (
        <span className="flex shrink-0 items-center gap-1">
          <button
            onClick={() => onDelete(c.id)}
            disabled={mutating}
            aria-label={`Permanently delete ${c.title}`}
            className="rounded-md px-1.5 py-0.5 text-sm font-medium text-danger transition hover:bg-danger/10"
          >
            Delete
          </button>
          <button
            onClick={() => setConfirming(false)}
            aria-label="Cancel delete"
            className="rounded-md px-1.5 py-0.5 text-sm text-ink-muted transition hover:bg-bg-hover hover:text-ink"
          >
            Cancel
          </button>
        </span>
      ) : (
          <button
            onClick={() => setConfirming(true)}
            disabled={mutating}
          title="Delete permanently"
          aria-label={`Delete ${c.title} permanently`}
          className="grid size-6 shrink-0 place-items-center rounded-md text-ink-faint opacity-0 transition hover:bg-danger/10 hover:text-danger group-hover:opacity-100"
        >
          <Trash2 className="size-3.5" />
        </button>
      )}
    </div>
  );
}

export const ArchivedRow = memo(
  ArchivedRowImpl,
  (a, b) =>
    a.c === b.c
    && a.mutation === b.mutation
    && a.onRestore === b.onRestore
    && a.onDelete === b.onDelete,
);
