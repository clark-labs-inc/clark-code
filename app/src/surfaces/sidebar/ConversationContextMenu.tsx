import { useEffect, useRef, useState } from "react";
import { Archive, Pencil, Trash2 } from "lucide-react";
import { handleMenuNavigation } from "../../lib/menuNavigation";

/** Right-click menu for one-or-many conversations. Acts on the whole sidebar
 *  selection when the right-clicked row is part of it, otherwise just the
 *  right-clicked row. "Archive" soft-deletes; "Delete" hard-deletes (with an
 *  inline confirm — it can't be undone). */
export function ConversationContextMenu({
  menu,
  count,
  onClose,
  onArchive,
  onDelete,
  onRename,
}: {
  menu: { x: number; y: number };
  count: number;
  onClose: () => void;
  onArchive: () => void;
  onDelete: () => void;
  onRename: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  const label = count > 1 ? `${count} conversations` : "conversation";
  return (
    <div
      ref={ref}
      onKeyDown={handleMenuNavigation}
      role="menu"
      style={{ left: menu.x, top: menu.y }}
      className="popover-surface fixed z-50 w-52 rounded-xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle"
    >
      {confirming ? (
        <div className="px-1.5 py-1">
          <div className="mb-2 px-1 text-sm text-ink-muted">
            Permanently delete {count > 1 ? `these ${count} conversations` : "this conversation"}? This can't be undone.
          </div>
          <div className="flex items-center gap-1.5">
            <button
              role="menuitem"
              autoFocus
              onClick={() => {
                onDelete();
                onClose();
              }}
              className="flex-1 rounded-lg bg-danger/10 px-2 py-1.5 text-sm font-medium text-danger transition hover:bg-danger/20"
            >
              Delete
            </button>
            <button
              role="menuitem"
              onClick={() => setConfirming(false)}
              className="flex-1 rounded-lg px-2 py-1.5 text-sm text-ink-muted transition hover:bg-bg-hover hover:text-ink"
            >
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <>
          {count === 1 && <button role="menuitem" autoFocus onClick={() => { onClose(); onRename(); }} className="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-base text-ink-secondary hover:bg-bg-hover"><Pencil className="size-4" />Rename conversation</button>}
          <button
            role="menuitem"
            autoFocus={count > 1}
            onClick={() => {
              onArchive();
              onClose();
            }}
            className="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-base text-ink-secondary transition hover:bg-accent-subtle hover:text-ink"
          >
            <Archive className="size-4" />
            Archive {label}
          </button>
          <button
            role="menuitem"
            onClick={() => setConfirming(true)}
            className="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-base text-ink-secondary transition hover:bg-danger/10 hover:text-danger"
          >
            <Trash2 className="size-4" />
            Delete {label}
          </button>
        </>
      )}
    </div>
  );
}

