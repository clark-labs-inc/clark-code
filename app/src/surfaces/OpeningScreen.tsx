import { Loader2, Server, MessageSquare } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";

/** Shown while a conversation is being (re)opened. Reopening a remote session
 *  re-establishes its SSH tunnel, which can take 10–20s — without this the app
 *  looked frozen on the start screen the whole time. */
export function OpeningScreen() {
  const opening = useSessionStore((s) => s.opening);
  if (!opening) return null;

  const Icon = opening.remoteHost ? Server : MessageSquare;
  return (
    <div className="fade-in flex flex-1 flex-col items-center justify-center gap-5 p-6">
      <span className="relative grid size-14 place-items-center">
        <Loader2 className="absolute size-14 animate-[spin_1s_linear_infinite] text-ink-faint/60" />
        <Icon className="size-5 text-ink-muted" />
      </span>
      <div className="max-w-md text-center">
        <p className="truncate text-base font-semibold text-ink">{opening.title}</p>
        <p className="mt-1.5 text-sm text-ink-muted">
          {opening.remoteHost
            ? `Reconnecting to ${opening.remoteHost}…`
            : "Opening session…"}
        </p>
      </div>
    </div>
  );
}
