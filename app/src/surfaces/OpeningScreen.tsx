import { Loader2, Server, MessageSquare, Laptop } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";

/** Full-pane feedback while a session connect is in flight — starting a new
 *  session or reopening one. Remote connects bring up an SSH tunnel (10–20s),
 *  so this owns the wait: what's happening, to which host, and a way out.
 *  The appear delay keeps sub-200ms local connects from flashing a spinner.
 *  (Peek fetches never show this screen — they only mark the sidebar row.) */
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
