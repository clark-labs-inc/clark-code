interface EarlierHistoryButtonProps {
  onClick: () => void;
}

export function EarlierHistoryButton({ onClick }: EarlierHistoryButtonProps) {
  return (
    <button
      onClick={onClick}
      className="mx-auto rounded-full border border-border-subtle bg-bg-elevated px-3.5 py-1.5 text-xs font-medium text-ink-muted transition hover:bg-bg-hover hover:text-ink-secondary"
    >
      Show earlier history
    </button>
  );
}

interface NewerHistoryControlsProps {
  onNewer: () => void;
  onLatest: () => void;
}

export function NewerHistoryControls({ onNewer, onLatest }: NewerHistoryControlsProps) {
  return (
    <div className="mx-auto flex items-center gap-2">
      <button
        onClick={onNewer}
        className="rounded-full border border-border-subtle bg-bg-elevated px-3.5 py-1.5 text-xs font-medium text-ink-muted transition hover:bg-bg-hover hover:text-ink-secondary"
      >
        Show newer history
      </button>
      <button
        onClick={onLatest}
        className="rounded-full px-3.5 py-1.5 text-xs font-medium text-ink-faint transition hover:bg-bg-hover hover:text-ink-secondary"
      >
        Back to latest
      </button>
    </div>
  );
}
