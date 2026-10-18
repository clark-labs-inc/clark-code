import { AnimatePresence } from "motion/react";
import * as m from "motion/react-m";
import { Archive, Trash2, X } from "lucide-react";
import type { RefObject } from "react";
import { RISE, accessibleMotion } from "../../lib/motion";

export function SidebarSelectionToolbar({ selectedCount, hasMutation, reduceMotion, deleteConfirming, deleteConfirmRef, selectionStatus, mutationInFlight, setDeleteConfirming, deleteSelectedWithFocus, archiveSelectedWithFocus, clearSelection }: {
  selectedCount: number;
  hasMutation: boolean;
  reduceMotion: boolean | null;
  deleteConfirming: boolean;
  deleteConfirmRef: RefObject<HTMLButtonElement | null>;
  selectionStatus: string;
  mutationInFlight: boolean;
  setDeleteConfirming: (value: boolean) => void;
  deleteSelectedWithFocus: () => void;
  archiveSelectedWithFocus: () => void;
  clearSelection: () => void;
}) {
  return (
        <AnimatePresence initial={false}>
          {(selectedCount > 0 || hasMutation) && (
            <m.div
              key="conversation-selection-toolbar"
              {...accessibleMotion(RISE, reduceMotion)}
              className="absolute inset-x-0 bottom-full z-10 bg-bg-secondary px-3 py-2 shadow-soft"
            >
              {deleteConfirming && selectedCount > 0 ? (
                <div className="flex items-center gap-1.5 text-sm text-ink-muted">
                  <span className="min-w-0 flex-1">Delete {selectedCount} permanently?</span>
                  <button
                    ref={deleteConfirmRef}
                    onClick={() => {
                      setDeleteConfirming(false);
                      deleteSelectedWithFocus();
                    }}
                    className="rounded-lg bg-danger/10 px-2 py-1.5 font-medium text-danger transition hover:bg-danger/20"
                  >
                    Delete
                  </button>
                  <button
                    onClick={() => setDeleteConfirming(false)}
                    className="rounded-lg px-2 py-1.5 text-ink-muted transition hover:bg-bg-hover hover:text-ink"
                  >
                    Cancel
                  </button>
                </div>
              ) : (
                <div className="flex items-center gap-1.5 text-sm text-ink-muted">
                  <span className="min-w-0 flex-1 tabular-nums">{selectionStatus}</span>
                  {selectedCount > 0 && (
                    <>
                      <button
                        disabled={mutationInFlight}
                        onClick={archiveSelectedWithFocus}
                        className="flex items-center gap-1.5 rounded-lg px-2 py-1.5 transition hover:bg-accent-subtle hover:text-ink disabled:cursor-wait disabled:opacity-50"
                        title="Archive selected"
                      >
                        <Archive className="size-3.5" /> Archive
                      </button>
                      <button
                        disabled={mutationInFlight}
                        onClick={() => setDeleteConfirming(true)}
                        className="flex items-center gap-1.5 rounded-lg px-2 py-1.5 transition hover:bg-danger/10 hover:text-danger disabled:cursor-wait disabled:opacity-50"
                        title="Delete selected permanently"
                      >
                        <Trash2 className="size-3.5" /> Delete
                      </button>
                      <button
                        disabled={mutationInFlight}
                        onClick={clearSelection}
                        aria-label="Clear selection"
                        title="Clear selection"
                        className="grid size-7 place-items-center rounded-lg text-ink-faint transition hover:bg-bg-hover hover:text-ink disabled:cursor-wait disabled:opacity-50"
                      >
                        <X className="size-3.5" />
                      </button>
                    </>
                  )}
                </div>
              )}
            </m.div>
          )}
        </AnimatePresence>
  );
}
