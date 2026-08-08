import { Loader2, Server, MessageSquare, Laptop } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";

/** Full-pane feedback only when no target conversation can be rendered yet.
 * Cached conversations stay visible while native reattachment runs in the
 * background, matching the persistent-host resume contract. */
export function OpeningScreen() {
  const opening = useSessionStore((s) => s.opening);
  const cancel = useSessionStore((s) => s.endSession);
  if (!opening) return null;

  const isStart = opening.kind === "start";
  const Icon = opening.remoteHost ? Server : isStart ? Laptop : MessageSquare;
  const status = opening.remoteHost
    ? `${isStart ? "Connecting" : "Reconnecting"} to ${opening.remoteHost}… this can take a moment`
    : isStart
      ? "Starting session…"
      : "Opening session…";

  return (
    <div className="appear-delayed flex flex-1 flex-col items-center justify-center gap-5 p-6">
      <span className="relative grid size-14 place-items-center">
        <Loader2 className="absolute size-14 animate-[spin_1s_linear_infinite] text-ink-faint/60" />
        <Icon className="size-5 text-ink-muted" />
      </span>
      <div className="max-w-md text-center">
        <p className="truncate text-base font-semibold text-ink">{opening.title}</p>
        <p className="mt-1.5 text-sm text-ink-muted">{status}</p>
      </div>
      <button
        onClick={() => cancel()}
        className="rounded-lg px-3 py-1.5 text-sm font-medium text-ink-muted transition hover:bg-bg-hover hover:text-ink"
      >
        Cancel
      </button>
    </div>
  );
}
