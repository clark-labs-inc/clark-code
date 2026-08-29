interface ComposerDraftConflictProps {
  onKeepCurrent: () => void;
  onUseSynced: () => void;
}

export function ComposerDraftConflict({
  onKeepCurrent,
  onUseSynced,
}: ComposerDraftConflictProps) {
  return (
    <div
      className="conversation-column-width mx-auto mt-2 flex min-h-8 w-full flex-wrap items-center gap-x-3 gap-y-1 px-1 text-xs"
      role="group"
      aria-label="Choose which draft to keep"
    >
      <p className="min-w-0 flex-1 text-ink-muted" role="status" aria-live="polite">
        Another saved draft is available.
      </p>
      <div className="flex shrink-0 items-center gap-1">
        <button
          type="button"
          onClick={onKeepCurrent}
          className="min-h-8 rounded-lg px-2.5 font-medium text-ink-secondary transition hover:bg-bg-hover hover:text-ink"
        >
          Keep current draft
        </button>
        <button
          type="button"
          onClick={onUseSynced}
          className="min-h-8 rounded-lg px-2.5 font-medium text-accent transition hover:bg-accent/10"
        >
          Use saved draft
        </button>
      </div>
    </div>
  );
}
