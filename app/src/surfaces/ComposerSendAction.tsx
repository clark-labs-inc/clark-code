import { ArrowUp, CornerDownRight, Loader2, Square } from "lucide-react";
import { cn } from "../lib/cn";

export function ComposerSendAction({
  submitting,
  busy,
  hasContent,
  canSend,
  shouldPickProjectFolder,
  startsScoutRun,
  queuedTitle,
  onCancel,
  onSubmit,
}: {
  submitting: boolean;
  busy: boolean;
  hasContent: boolean;
  canSend: boolean;
  shouldPickProjectFolder: boolean;
  startsScoutRun: boolean;
  queuedTitle: string;
  onCancel: () => void;
  onSubmit: () => void;
}) {
  if (submitting) {
    return (
      <button
        type="button"
        disabled
        aria-label="Sending message"
        aria-busy="true"
        className="grid size-8 shrink-0 place-items-center rounded-full bg-accent text-on-accent shadow-soft"
      >
        <Loader2 aria-hidden="true" className="size-4 animate-spin" />
      </button>
    );
  }

  if (busy && !hasContent) {
    return (
      <button
        type="button"
        onClick={onCancel}
        aria-label="Stop"
        className="grid size-8 shrink-0 place-items-center rounded-full bg-danger/12 text-danger transition duration-base ease-agent hover:bg-danger/20"
      >
        <Square aria-hidden="true" className="size-3 fill-current" />
      </button>
    );
  }

  const label = shouldPickProjectFolder
    ? "Choose project folder and send"
    : busy
      ? "Queue message"
      : startsScoutRun
        ? "Start Scout run"
        : "Send";
  const title = shouldPickProjectFolder
    ? "Choose project folder and send"
    : busy
      ? queuedTitle
      : startsScoutRun
        ? "Start Scout run · human initiated"
        : "Send · ⇧↵ newline";

  return (
    <button
      type="button"
      onClick={onSubmit}
      disabled={!canSend}
      aria-label={label}
      title={title}
      className={cn(
        "shrink-0 bg-accent text-on-accent shadow-soft transition duration-base ease-agent hover:-translate-y-0.5 hover:bg-accent-hover active:translate-y-0 disabled:translate-y-0 disabled:bg-bg-tertiary disabled:text-ink-muted disabled:shadow-none",
        startsScoutRun
          ? "inline-flex h-8 items-center gap-1.5 rounded-full px-3 text-xs font-semibold"
          : "grid size-8 place-items-center rounded-full",
      )}
    >
      {busy
        ? <CornerDownRight aria-hidden="true" className="size-4" />
        : startsScoutRun
          ? <><ArrowUp aria-hidden="true" className="size-3.5" /><span>Start run</span></>
          : <ArrowUp aria-hidden="true" className="size-4" />}
    </button>
  );
}
